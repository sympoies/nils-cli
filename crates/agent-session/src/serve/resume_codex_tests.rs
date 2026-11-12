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
        json!({"agent":"codex","resume":ID,"account":"original","resume_account_explicit":false,"id":"blocked"})).await;
    assert!(!status.is_success(), "{result}");
    assert_eq!(result["error"]["code"], "resume-account-mismatch");
    assert_eq!(
        result["error"]["details"],
        json!({"session_created":false,"safe_to_retry":false})
    );
    assert_eq!(snapshot(temp.path()), before);
}

#[test]
fn console_codex_explicit_account_reaches_preflight_request() {
    let session = json!({"agent":"codex", "resume":ID, "account":"original"});
    let request = crate::coordination::resume_preflight::request_from_session(&session).unwrap();
    assert_eq!(request.account.as_deref(), Some("original"));
}

fn unbound_codex_fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let (state, broker, tmux) = fixture(root);
    let path = state.join("sessions/source/session.json");
    let mut source: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    source["agent"] = json!("codex");
    source["provider_resume"]["provider"] = json!("codex");
    source
        .as_object_mut()
        .unwrap()
        .remove("claude_account_binding");
    fs::write(path, source.to_string()).unwrap();
    (state, broker, tmux)
}

#[tokio::test]
async fn console_codex_explicit_account_reaches_target_admission() {
    let temp = tempfile::tempdir().unwrap();
    let (state, _, tmux) = unbound_codex_fixture(temp.path());
    let st = crate::serve::tests::state(&state, Some(TOKEN), tmux);
    let (_, result) = post(
        router(st),
        "/sessions/resume/v1",
        Some(TOKEN),
        json!({"agent":"codex","resume":ID,"account":"original"}),
    )
    .await;
    assert_eq!(result["error"]["code"], "resume-source-unknown");
}

#[tokio::test]
async fn sessions_list_advertises_shared_account_field() {
    let temp = tempfile::tempdir().unwrap();
    let state =
        crate::serve::tests::state(temp.path(), Some(TOKEN), PathBuf::from("/unavailable/tmux"));
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["data"]["capabilities"]["session_account_field"], true);
}

#[tokio::test]
async fn target_resume_accepts_both_account_fields_and_rejects_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let (state, _, tmux) = unbound_codex_fixture(temp.path());
    let app = router(crate::serve::tests::state(&state, Some(TOKEN), tmux));
    for route in ["/resume-preflight/v1", "/sessions/resume/v1"] {
        for field in ["account", "codex_account"] {
            let (_, result) = post(
                app.clone(),
                route,
                Some(TOKEN),
                json!({"agent":"codex","resume":ID,field:"original"}),
            )
            .await;
            assert_eq!(
                result["error"]["code"], "resume-source-unknown",
                "{route} {field}: {result}"
            );
        }
        for fields in [
            json!({"account":"original","codex_account":"original"}),
            json!({"claude_account":"original"}),
        ] {
            let mut body = json!({"agent":"codex","resume":ID});
            body.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let (_, result) = post(app.clone(), route, Some(TOKEN), body).await;
            assert_eq!(
                result["error"]["code"], "resume-unsupported",
                "{route}: {result}"
            );
        }
    }
}

#[tokio::test]
async fn fresh_create_accepts_both_account_fields_and_rejects_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let app = router(crate::serve::tests::state(
        temp.path(),
        Some(TOKEN),
        PathBuf::from("/unavailable/tmux"),
    ));
    for field in ["account", "codex_account"] {
        let (_, result) = post(
            app.clone(),
            "/sessions",
            Some(TOKEN),
            json!({"agent":"codex",field:"invalid/name"}),
        )
        .await;
        assert_eq!(
            result["error"]["code"], "invalid-codex-account",
            "{field}: {result}"
        );
    }
    let (_, result) = post(
        app.clone(),
        "/sessions",
        Some(TOKEN),
        json!({"agent":"codex","account":"original","codex_account":"original"}),
    )
    .await;
    assert_eq!(result["error"]["code"], "invalid-account");
    let (_, result) = post(
        app,
        "/sessions",
        Some(TOKEN),
        json!({"agent":"codex","claude_account":"original"}),
    )
    .await;
    assert_eq!(result["error"]["code"], "claude-account-agent-conflict");
}

#[tokio::test]
async fn create_routes_reject_null_account_key_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let (state, _, tmux) = unbound_codex_fixture(temp.path());
    let app = router(crate::serve::tests::state(&state, Some(TOKEN), tmux));
    for route in ["/sessions", "/sessions/resume/v1", "/resume-preflight/v1"] {
        for fields in [
            json!({"account":"original","codex_account":null}),
            json!({"account":null,"codex_account":"original"}),
            json!({"claude_account":null}),
        ] {
            let mut body = json!({"agent":"codex"});
            if route != "/sessions" {
                body["resume"] = json!(ID);
            }
            body.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let (status, result) = post(app.clone(), route, Some(TOKEN), body).await;
            if route == "/sessions" {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{result}");
            } else {
                assert_eq!(result["error"]["code"], "resume-unsupported", "{result}");
            }
        }
    }
}
