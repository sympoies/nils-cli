use super::*;
use pretty_assertions::assert_eq;
use std::time::SystemTime;

#[tokio::test]
async fn app_server_thread_binding_persists_provider_resume_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let (_socket_dir, socket) = nils_test_support::short_socket_path("codex.sock");
    let record = record_with_runtime("thread-resume-identity", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    bind_thread(&record, "raw-recoverable-thread").unwrap();
    persist_thread_resume(&context, &record, "raw-recoverable-thread")
        .await
        .unwrap();

    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let resume = persisted
        .provider_resume
        .as_ref()
        .expect("app-server binding must persist provider resume metadata");
    assert_eq!(resume.provider, "codex");
    assert_eq!(resume.session_id, "raw-recoverable-thread");
    assert_eq!(resume.capture_method, "codex-app-server-thread-binding");
    assert_eq!(
        resume.resume_args,
        crate::canonical_provider_resume_args(
            crate::AgentKind::Codex,
            &record.cwd,
            "raw-recoverable-thread",
        )
        .unwrap()
    );

    let sidecar = fs::read_to_string(
        crate::session_dir(&context, &record.id).join(crate::SESSION_RESUME_FILE),
    )
    .unwrap();
    assert!(sidecar.contains("raw-recoverable-thread"));
    let attached = fs::read_to_string(socket.with_extension("attached")).unwrap();
    assert_eq!(attached, projected_thread_binding("raw-recoverable-thread"));
    assert!(!attached.contains("raw-recoverable-thread"));
}

#[test]
fn app_server_thread_binding_preserves_matching_resume_provenance() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let (_socket_dir, socket) = nils_test_support::short_socket_path("codex.sock");
    let expected = record_with_runtime("thread-resume-preserved", &socket);
    let mut current = expected.clone();
    current.provider_resume = Some(ProviderResume {
        provider: "codex".to_string(),
        session_id: "raw-existing-thread".to_string(),
        captured_at: "2030-01-01T00:00:01Z".to_string(),
        capture_method: "codex-user-prompt-submit-hook".to_string(),
        resume_args: canonical_provider_resume_args(
            AgentKind::Codex,
            &current.cwd,
            "raw-existing-thread",
        )
        .unwrap(),
        extra: BTreeMap::from([("retained".to_string(), json!(true))]),
    });
    fs::create_dir_all(crate::session_dir(&context, &current.id)).unwrap();
    crate::write_session_record(&context, &current).unwrap();

    bind_thread(&current, "raw-existing-thread").unwrap();
    persist_bound_thread_resume(&context, &expected, "raw-existing-thread").unwrap();

    let persisted = crate::load_session_record(&context, &expected.id).unwrap();
    let resume = persisted.provider_resume.expect("provider resume");
    assert_eq!(resume.capture_method, "codex-user-prompt-submit-hook");
    assert_eq!(resume.extra.get("retained"), Some(&json!(true)));
    assert!(socket.with_extension("attached").is_file());
}

#[test]
fn app_server_thread_binding_rejects_conflicting_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let (_socket_dir, socket) = nils_test_support::short_socket_path("codex.sock");
    let expected = record_with_runtime("thread-resume-conflict", &socket);
    let mut current = expected.clone();
    current.provider_resume = Some(ProviderResume {
        provider: "codex".to_string(),
        session_id: "raw-existing-thread".to_string(),
        captured_at: "2030-01-01T00:00:01Z".to_string(),
        capture_method: "codex-user-prompt-submit-hook".to_string(),
        resume_args: canonical_provider_resume_args(
            AgentKind::Codex,
            &current.cwd,
            "raw-existing-thread",
        )
        .unwrap(),
        extra: BTreeMap::new(),
    });
    fs::create_dir_all(crate::session_dir(&context, &current.id)).unwrap();
    crate::write_session_record(&context, &current).unwrap();

    bind_thread(&current, "raw-different-thread").unwrap();
    let before = binding_documents(&context, &current);
    let error =
        persist_bound_thread_resume(&context, &expected, "raw-different-thread").unwrap_err();

    assert_eq!(error.code(), "codex-app-server-resume-identity-conflict");
    assert_eq!(binding_documents(&context, &current), before);
    assert_eq!(
        guarded::Client::attached_thread(&current)
            .unwrap_err()
            .code(),
        "send-identity-mismatch"
    );
    let persisted = crate::load_session_record(&context, &expected.id).unwrap();
    assert_eq!(
        persisted
            .provider_resume
            .as_ref()
            .map(|resume| resume.session_id.as_str()),
        Some("raw-existing-thread")
    );
}

#[test]
fn stale_app_server_runtime_cannot_bind_thread_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let (_socket_dir, socket) = nils_test_support::short_socket_path("codex.sock");
    let expected = record_with_runtime("thread-resume-stale", &socket);
    let mut replacement = expected.clone();
    replacement.runtime.as_mut().expect("runtime").launch_id = "replacement-runtime".to_string();
    fs::create_dir_all(crate::session_dir(&context, &replacement.id)).unwrap();
    crate::write_session_record(&context, &replacement).unwrap();

    bind_thread(&expected, "raw-stale-thread").unwrap();
    let before = binding_documents(&context, &replacement);
    let error = persist_bound_thread_resume(&context, &expected, "raw-stale-thread").unwrap_err();

    assert_eq!(error.code(), "session-runtime-changed");
    assert_eq!(binding_documents(&context, &replacement), before);
    let persisted = crate::load_session_record(&context, &expected.id).unwrap();
    assert!(persisted.provider_resume.is_none());
}

const PRIMARY: &str = "primary-thread";
const SIBLING: &str = "same-directory-sibling";

fn binding_documents(context: &CliContext, record: &SessionRecord) -> [Option<Vec<u8>>; 2] {
    let dir = crate::session_dir(context, &record.id);
    ["session.json", crate::SESSION_RESUME_FILE].map(|name| match fs::read(dir.join(name)) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("could not read {name}: {error}"),
    })
}

fn persisted_fixture(root: &Path, record: &SessionRecord) -> CliContext {
    let context = CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, record).unwrap();
    context
}

fn history_fixture(root: &Path, record: &mut SessionRecord, id: &str) -> SystemTime {
    let now = Timestamp::now();
    let launch_started_at = SystemTime::from(now) - Duration::from_secs(1);
    record.created_at = Timestamp::try_from(launch_started_at).unwrap().to_string();
    let profile = root.join("provider");
    let runtime = record.runtime.as_mut().unwrap();
    runtime.extra.insert(
        crate::AGENT_PROFILE_RUNTIME_KEY.into(),
        json!("codex-fixture"),
    );
    runtime.extra.insert(
        crate::AGENT_PROFILE_PROVIDER_CONFIG_DIR_RUNTIME_KEY.into(),
        json!(profile),
    );
    let history = profile.join("sessions");
    fs::create_dir_all(&history).unwrap();
    fs::write(
        history.join("rollout.jsonl"),
        json!({
            "type": "session_meta",
            "payload": {"id": id, "cwd": record.cwd, "source": "cli", "timestamp": now.to_string()}
        })
        .to_string(),
    )
    .unwrap();
    launch_started_at
}

#[test]
fn app_server_launch_binds_its_attached_thread_despite_same_cwd_history() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut record = record_with_runtime("console-recipient", &tmp.path().join("app.sock"));
    let launch_started_at = history_fixture(tmp.path(), &mut record, SIBLING);
    let context = persisted_fixture(tmp.path(), &record);
    bind_thread(&record, PRIMARY).unwrap();

    crate::capture_and_persist_provider_resume_after_launch(
        &context,
        AgentKind::Codex,
        &mut record,
        launch_started_at,
    )
    .unwrap();
    let selected = attached_loaded_thread(&record, &[PRIMARY.into()])
        .unwrap()
        .unwrap();
    persist_bound_thread_resume(&context, &record, &selected).unwrap();

    let current = crate::load_session_record(&context, &record.id).unwrap();
    let resume = current.provider_resume.unwrap();
    assert_eq!(resume.session_id, PRIMARY);
    assert_eq!(resume.capture_method, PROVIDER_RESUME_CAPTURE_METHOD);
}

#[test]
fn app_server_history_backfill_cannot_bind_same_cwd_history() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut record = record_with_runtime("console-backfill", &tmp.path().join("app.sock"));
    record.runtime.as_mut().unwrap().generation = 2;
    history_fixture(tmp.path(), &mut record, SIBLING);
    let context = persisted_fixture(tmp.path(), &record);
    let before = binding_documents(&context, &record);

    assert!(crate::capture_codex_resume_from_history(&record).is_none());
    let current = crate::backfill_provider_resume(&context, record);
    assert!(current.provider_resume.is_none());
    assert_eq!(binding_documents(&context, &current), before);
}

#[test]
fn app_server_single_loaded_thread_without_attachment_cannot_bind() {
    let tmp = tempfile::TempDir::new().unwrap();
    let record = record_with_runtime("no-attachment", &tmp.path().join("app.sock"));
    let context = persisted_fixture(tmp.path(), &record);
    let before = binding_documents(&context, &record);

    let loaded = loaded_thread_ids(&json!({"data": [PRIMARY]})).unwrap();
    assert_eq!(attached_loaded_thread(&record, &loaded).unwrap(), None);
    let error = persist_bound_thread_resume(&context, &record, &loaded[0]).unwrap_err();

    assert_eq!(error.code(), "codex-app-server-thread-binding-failed");
    assert!(!thread_attached_path(&record).unwrap().exists());
    assert_eq!(binding_documents(&context, &record), before);
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        guarded::Client::attached_thread(&current)
            .unwrap_err()
            .code(),
        "send-identity-unobservable"
    );
}

#[test]
fn app_server_changed_attachment_after_selection_cannot_bind() {
    let tmp = tempfile::TempDir::new().unwrap();
    let record = record_with_runtime("changed-attachment", &tmp.path().join("app.sock"));
    let context = persisted_fixture(tmp.path(), &record);
    let before = binding_documents(&context, &record);
    bind_thread(&record, PRIMARY).unwrap();
    let selected = attached_loaded_thread(&record, &[PRIMARY.into()])
        .unwrap()
        .unwrap();
    replace_thread_binding(&record, SIBLING).unwrap();

    let error = persist_bound_thread_resume(&context, &record, &selected).unwrap_err();

    assert_eq!(error.code(), "codex-app-server-thread-binding-failed");
    assert_eq!(binding_documents(&context, &record), before);
    assert_eq!(
        fs::read_to_string(thread_attached_path(&record).unwrap()).unwrap(),
        projected_thread_binding(SIBLING)
    );
}

#[test]
fn cli_launch_can_capture_its_own_matching_history() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut record = record_with_runtime("standalone", &tmp.path().join("unused.sock"));
    record.runtime.as_mut().unwrap().kind = "tmux".into();
    let launch_started_at = history_fixture(tmp.path(), &mut record, PRIMARY);
    let context = persisted_fixture(tmp.path(), &record);

    crate::capture_and_persist_provider_resume_after_launch(
        &context,
        AgentKind::Codex,
        &mut record,
        launch_started_at,
    )
    .unwrap();

    let current = crate::load_session_record(&context, &record.id).unwrap();
    let resume = current.provider_resume.unwrap();
    assert_eq!(resume.session_id, PRIMARY);
    assert_eq!(resume.capture_method, "codex-session-meta");
}
