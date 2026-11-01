use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

fn record(root: &Path, id: &str) -> SessionRecord {
    serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1", "id":"synthetic", "agent":"codex", "mode":"oneshot", "title":null,
        "cwd":"/workspace/project", "tmux_session":"synthetic", "prompt_file":null, "log_file":null,
        "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z",
        "provider_resume":{"provider":"codex", "session_id":id, "captured_at":"2026-01-01T00:00:00Z", "capture_method":"fixture", "resume_args":[]},
        "runtime":{"kind":"tmux", "tmux_session":"synthetic", "generation":1, "started_at":"2026-01-01T00:00:00Z", "launch_id":"synthetic-runtime", "agent_profile_provider_config_dir":root}
    })).unwrap()
}

fn history(root: &Path, name: &str, id: &str) -> PathBuf {
    fs::create_dir_all(root.join("sessions")).unwrap();
    let path = root.join("sessions").join(name);
    fs::write(&path, format!("{}\n", json!({"type":"session_meta", "payload":{
        "id":id, "cwd":"/workspace/project", "source":"cli", "timestamp":"2026-01-01T00:00:00Z"
    }}))).unwrap();
    path
}

#[test]
fn one_discovery_per_scope_and_new_queries_rediscover_missing_and_duplicate_ids() {
    let lock = nils_test_support::GlobalStateLock::new();
    let _deadline =
        nils_test_support::EnvGuard::set(&lock, "AGENT_SESSION_CODEX_SCAN_SLICE_MS", "60000");
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..1000 {
        history(
            tmp.path(),
            &format!("arbitrary-{i}.jsonl"),
            &format!("provider-{i}"),
        );
    }
    let mut query = QuerySources::default();
    for i in 0..25 {
        let mut row = record(tmp.path(), &format!("provider-{i}"));
        row.id = format!("session-{i}");
        row.runtime.as_mut().unwrap().launch_id = format!("runtime-{i}");
        assert!(query.codex_source(&row).unwrap().validated_path().is_some());
    }
    assert_eq!(
        query.scans, 1,
        "equivalent roots must not be scanned once per session"
    );
    let unknown = record(tmp.path(), "missing");
    assert!(query.codex_source(&unknown).is_none());
    history(tmp.path(), "new.jsonl", "missing");
    assert!(
        QuerySources::default().codex_source(&unknown).is_some(),
        "a later query must rediscover"
    );
    history(tmp.path(), "duplicate.jsonl", "provider-0");
    assert!(
        QuerySources::default()
            .codex_source(&record(tmp.path(), "provider-0"))
            .is_none()
    );
}

#[test]
fn current_root_profile_provider_and_resume_bindings_select_their_own_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let first = tmp.path().join("first");
    let second = tmp.path().join("second");
    let first_path = history(&first, "arbitrary.jsonl", "same-id");
    let second_path = history(&second, "other.jsonl", "same-id");
    let mut query = QuerySources::default();
    let mut row = record(&first, "same-id");
    assert_eq!(query.codex_source(&row).unwrap().path(), first_path);
    row.runtime
        .as_mut()
        .unwrap()
        .extra
        .insert("agent_profile_provider_config_dir".into(), json!(second));
    assert_eq!(query.codex_source(&row).unwrap().path(), second_path);
    assert_eq!(query.scans, 2);
    row.runtime
        .as_mut()
        .unwrap()
        .extra
        .insert("agent_profile".into(), json!("another-profile"));
    assert_eq!(query.codex_source(&row).unwrap().path(), second_path);
    assert_eq!(
        query.scans, 3,
        "different profile binding must not reuse scope"
    );
    row.provider_resume.as_mut().unwrap().session_id = "missing".into();
    assert!(
        query.codex_source(&row).is_none(),
        "current resume id must be used"
    );
    row.provider_resume.as_mut().unwrap().session_id = "same-id".into();
    row.provider_resume.as_mut().unwrap().provider = "claude".into();
    assert!(query.codex_source(&row).is_none());
    row.agent = "claude".into();
    assert!(query.codex_source(&row).is_none());
    assert_eq!(
        query.scans, 3,
        "foreign providers must not consult the Codex snapshot"
    );
}

#[test]
fn chosen_source_is_revalidated_after_replacement_or_removal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = history(tmp.path(), "arbitrary.jsonl", "target");
    let row = record(tmp.path(), "target");
    let mut query = QuerySources::default();
    assert!(query.codex_source(&row).unwrap().validated_path().is_some());
    let replacement = history(tmp.path(), "replacement.jsonl", "foreign");
    fs::rename(replacement, &path).unwrap();
    assert!(query.codex_source(&row).unwrap().validated_path().is_none());
    fs::remove_file(path).unwrap();
    assert!(query.codex_source(&row).unwrap().validated_path().is_none());
    assert_eq!(query.scans, 1);
}
