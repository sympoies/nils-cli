use super::*;
use pretty_assertions::assert_eq;

const PROVIDER_ID: &str = "12345678-1234-1234-1234-123456789abc";

fn registry_record(state: &Path) -> PathBuf {
    let dir = state.join("sessions/managed-fixture");
    fs::create_dir_all(&dir).unwrap();
    let record = serde_json::json!({
        "schema_version": crate::SESSION_DOCUMENT_VERSION,
        "id": "managed-fixture", "agent": "claude", "mode": "interactive",
        "title": null, "cwd": "/work/repo", "tmux_session": "fixture",
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "provider_resume": {
            "provider": "claude", "session_id": PROVIDER_ID,
            "captured_at": "2026-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": []
        }
    });
    let path = dir.join("session.json");
    fs::write(&path, record.to_string()).unwrap();
    path
}

fn source(root: PathBuf, profile: Option<&str>) -> HistorySource {
    fs::create_dir_all(root.join("repo")).unwrap();
    fs::write(
        root.join("repo").join(format!("{PROVIDER_ID}.jsonl")),
        format!("{{\"sessionId\":\"{PROVIDER_ID}\",\"cwd\":\"/work/repo\"}}\n"),
    )
    .unwrap();
    HistorySource {
        provider: "claude".into(),
        agent_profile: profile.map(str::to_string),
        root,
    }
}

fn saved_overlays(root: &Path) -> String {
    let archives = root.join("archives");
    let stars = root.join("stars");
    let id = stable_history_id("claude", Some("profile-a"), PROVIDER_ID);
    write_archive(
        &archives,
        &ArchivedSession {
            title_mode: Default::default(),
            schema_version: "agent-session.history-archive.v1".into(),
            history_id: id.clone(),
            provider: "claude".into(),
            provider_session_id: PROVIDER_ID.into(),
            agent_profile: Some("profile-a".into()),
            session_id: Some("managed-fixture".into()),
            title: Some("Saved title".into()),
            title_state: None,
            cwd: "/work/repo".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            archived_at: "2026-01-02T00:00:00Z".into(),
        },
    )
    .unwrap()
    .commit();
    write_star(&stars, &id, "2026-01-03T00:00:00Z").unwrap();
    id
}

#[test]
fn seeded_sessions_keep_source_identity_overlays_and_do_not_duplicate() {
    let tmp = tempfile::tempdir().unwrap();
    registry_record(tmp.path());
    let sources = vec![
        source(tmp.path().join("first"), Some("profile-a")),
        source(tmp.path().join("second"), Some("profile-b")),
    ];
    let archives = tmp.path().join("archives");
    let stars = tmp.path().join("stars");
    let id = saved_overlays(tmp.path());
    let catalog =
        HistoryCatalog::new(sources, archives, stars).with_session_state_dir(tmp.path().into());
    let page = catalog.list("test", None, None, None, 50).unwrap();
    assert!(!page.truncated);
    assert_eq!(page.sessions.len(), 2);
    assert_eq!(page.sessions[0].id, id);
    assert_eq!(page.sessions[0].title.as_deref(), Some("Saved title"));
    assert_eq!(
        page.sessions[0].archived_session_id.as_deref(),
        Some("managed-fixture")
    );
    assert!(page.sessions[0].starred_at.is_some());
    assert_eq!(page.sessions[1].agent_profile.as_deref(), Some("profile-b"));
    assert!(page.sessions[1].title.is_none());
    assert!(page.sessions[1].archived_at.is_none());
    assert_eq!(catalog.snapshot().incremental_segments[&id].len(), 1);
}

#[test]
fn managed_probes_share_entry_and_deadline_limits() {
    let tmp = tempfile::tempdir().unwrap();
    registry_record(tmp.path());
    let sources = [source(tmp.path().join("projects"), None)];
    for (mut visited, deadline) in [
        (SCAN_MAX_ENTRIES, Instant::now() + SCAN_MAX_DURATION),
        (0, Instant::now() - Duration::from_secs(1)),
    ] {
        let mut truncated = false;
        let sessions = managed_claude_sessions(
            &sources,
            Some(tmp.path()),
            deadline,
            &mut visited,
            &mut truncated,
        );
        assert!(sessions.is_empty());
        assert!(truncated);
        assert!(visited <= SCAN_MAX_ENTRIES);
    }
}

#[test]
fn managed_priority_rejects_linked_or_unbounded_registry_inputs_and_transcripts() {
    for invalid in [
        "record-link",
        "record-size",
        "sidecar-link",
        "project-link",
        "transcript-link",
        "identity",
        "invalid-id",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let record_path = registry_record(tmp.path());
        let src = source(tmp.path().join("projects"), None);
        let transcript = src.root.join("repo").join(format!("{PROVIDER_ID}.jsonl"));
        match invalid {
            "record-link" | "sidecar-link" | "project-link" | "transcript-link" => {
                let path = match invalid {
                    "record-link" => record_path.clone(),
                    "sidecar-link" => record_path.with_file_name(crate::SESSION_RESUME_FILE),
                    "project-link" => src.root.join("repo"),
                    _ => transcript,
                };
                let target = tmp.path().join("linked-target");
                if path.exists() {
                    fs::rename(&path, &target).unwrap();
                } else {
                    fs::write(&target, "{}").unwrap();
                }
                std::os::unix::fs::symlink(target, path).unwrap();
            }
            "record-size" => fs::File::create(&record_path)
                .unwrap()
                .set_len(1024 * 1024 + 1)
                .unwrap(),
            "identity" | "invalid-id" => {
                let mut record: Value =
                    serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
                if invalid == "identity" {
                    record["id"] = "different-record".into();
                } else {
                    record["provider_resume"]["session_id"] = "../../outside".into();
                }
                fs::write(record_path, record.to_string()).unwrap();
            }
            _ => unreachable!(),
        }
        let mut truncated = false;
        let sessions = managed_claude_sessions(
            &[src],
            Some(tmp.path()),
            Instant::now() + SCAN_MAX_DURATION,
            &mut 0,
            &mut truncated,
        );
        assert!(sessions.is_empty(), "accepted {invalid}");
    }
}

// Advance only this test thread's scan deadline at the managed-probe boundary.
// No wall-clock sleeps or large/slow fixtures are needed to exercise cutoff.
thread_local! {
    static EXPIRE_AFTER_MANAGED_PROBES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn after_managed_probes(deadline: Instant) -> Instant {
    EXPIRE_AFTER_MANAGED_PROBES.with(|expire| {
        if expire.replace(false) {
            Instant::now() - Duration::from_secs(1)
        } else {
            deadline
        }
    })
}

#[test]
fn managed_probe_deadline_preserves_saved_overlays() {
    let tmp = tempfile::tempdir().unwrap();
    registry_record(tmp.path());
    let sources = vec![source(tmp.path().join("projects"), Some("profile-a"))];
    let id = saved_overlays(tmp.path());
    let catalog = HistoryCatalog::new(
        sources,
        tmp.path().join("archives"),
        tmp.path().join("stars"),
    )
    .with_session_state_dir(tmp.path().into());
    EXPIRE_AFTER_MANAGED_PROBES.with(|expire| expire.set(true));
    let page = catalog.list("test", None, None, None, 50).unwrap();
    assert!(page.truncated);
    assert_eq!(page.sessions.len(), 1);
    let session = &page.sessions[0];
    assert_eq!(session.id, id);
    assert_eq!(session.title.as_deref(), Some("Saved title"));
    assert_eq!(
        session.archived_session_id.as_deref(),
        Some("managed-fixture")
    );
    assert_eq!(session.archived_at.as_deref(), Some("2026-01-02T00:00:00Z"));
    assert_eq!(session.starred_at.as_deref(), Some("2026-01-03T00:00:00Z"));
}
