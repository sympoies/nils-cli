//! Codex shares the authenticated target transport and its terminal refusals.
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn codex_target_preflight_is_read_only_and_create_rechecks_the_pinned_account() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let (state, broker, tmux) = fixture(temp.path());
    let config = temp.path().join("account");
    fs::create_dir_all(config.join("sessions/2026/01/01")).unwrap();
    fs::write(config.join(format!("sessions/2026/01/01/rollout-{ID}.jsonl")),
        json!({"type":"session_meta","payload":{"id":ID,"cwd":temp.path(),"source":"cli","timestamp":"2026-01-01T00:00:00Z"}}).to_string()).unwrap();
    let source_path = state.join("sessions/source/session.json");
    let mut source: Value = serde_json::from_slice(&fs::read(&source_path).unwrap()).unwrap();
    source["agent"] = json!("codex");
    source["provider_resume"]["provider"] = json!("codex");
    source["codex_account_binding"] = json!({"schema_version":"agent-session.codex-account-binding.v1",
        "selected_account":"original","revision":1,"state":"bound","applied_runtime_id":"source-runtime","updated_at":"2026-01-01T00:00:00Z"});
    source
        .as_object_mut()
        .unwrap()
        .remove("claude_account_binding");
    fs::write(&source_path, source.to_string()).unwrap();
    executable(
        &broker,
        "#!/bin/sh\n[ \"$1\" = list ] || exit 64\necho '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"accounts\":[{\"account\":\"original\"}]}'\n",
    );
    let agent = temp.path().join("codex");
    executable(
        &agent,
        "#!/bin/sh\ncase \"$*\" in --version) echo 'codex-cli 0.145.0';; 'app-server --help') echo '--listen <URL> unix://PATH';; *) exit 64;; esac\n",
    );
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &json!([broker]).to_string(),
    );
    let _agent = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", agent.to_str().unwrap());
    let _root = EnvGuard::set(&lock, "CODEX_HOME", config.to_str().unwrap());
    let _runtime = EnvGuard::set(&lock, "XDG_RUNTIME_DIR", temp.path().to_str().unwrap());
    let st = crate::serve::tests::state(&state, Some(TOKEN), tmux);
    let before = snapshot(temp.path());
    let (status, checked) = post(
        router(st.clone()),
        "/resume-preflight/v1",
        Some(TOKEN),
        json!({"agent":"codex","resume":ID}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    assert_eq!(
        checked,
        json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,
        "data":{"agent":"codex","provider_session_id":ID,"machine":"test-machine","account":"original"}})
    );
    assert_eq!(snapshot(temp.path()), before);

    // The Console's resolved pin uses the shared resume transport account
    // field. A later binding change must refuse before an operation or record.
    source["codex_account_binding"]["selected_account"] = json!("changed");
    fs::write(&source_path, source.to_string()).unwrap();
    let before = snapshot(temp.path());
    let (status, result) = post(router(st), "/sessions/resume/v1", Some(TOKEN),
        json!({"agent":"codex","resume":ID,"claude_account":"original","resume_account_explicit":false,"id":"blocked"})).await;
    assert!(!status.is_success(), "{result}");
    assert_eq!(result["error"]["code"], "resume-account-mismatch");
    assert_eq!(
        result["error"]["details"],
        json!({"session_created":false,"safe_to_retry":false})
    );
    assert_eq!(snapshot(temp.path()), before);
}
