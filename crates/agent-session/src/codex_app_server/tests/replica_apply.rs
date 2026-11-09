use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn rotated_replica_apply_reports_login_rejection_and_retries_current_credential() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let credential = tmp.path().join("replica-credential.json");
    let write_credential = |token: &str| {
        fs::write(
            &credential,
            json!({
                "schema_version":"agent-session.codex-auth-broker.v1",
                "account":"acct1", "access_token":token,
                "chatgpt_account_id":"fixture-workspace", "plan":"team"
            })
            .to_string(),
        )
        .unwrap();
    };
    write_credential("rotated-fixture-token");
    let broker = tmp.path().join("broker");
    fs::write(&broker, "#!/bin/sh\ncat \"$1\"\n").unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&[broker, credential.clone()]).unwrap(),
    );
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let (_socket_dir, path) = nils_test_support::short_socket_path("replica-apply.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut record = record_with_runtime("replica-apply", &path);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        for token in ["rotated-fixture-token", "newer-fixture-token"] {
            let login = receive_json(&mut socket).await;
            assert_eq!(login["method"], "account/login/start");
            assert_eq!(login["params"]["accessToken"], token);
            if token == "rotated-fixture-token" {
                send_json(&mut socket, json!({"id":login["id"],
                    "error":{"code":-32603,"message":"private-fixture-error rotated-fixture-token"}}))
                    .await.unwrap();
            } else {
                respond(&mut socket, &login, json!({"type":"chatgptAuthTokens"})).await;
            }
        }
    });
    let stream = UnixStream::connect(&path).await.unwrap();
    let (mut socket, _) = tokio_tungstenite::client_async("ws://localhost", stream)
        .await
        .unwrap();
    let mut request_id = 0;
    let error = apply_account_binding(&mut socket, &context, &record, &mut request_id, "acct1", 1)
        .await
        .unwrap_err();
    assert_eq!(error, "Codex external-auth login failed: login_rejected");
    // The daemon's failure completion must not erase the control's diagnosis.
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-replica-apply",
        "acct1",
        1,
        Err("control_apply_unconfirmed"),
    )
    .unwrap();
    let failed = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&failed);
    assert_eq!(view.failure_reason.as_deref(), Some("login_rejected"));
    assert!(crate::codex_account::ensure_input_allowed(&failed).is_err());
    let persisted = serde_json::to_string(&failed).unwrap();
    assert!(!persisted.contains("rotated-fixture-token"));
    assert!(!persisted.contains("private-fixture-error"));

    write_credential("newer-fixture-token");
    let mut replacement = failed.clone();
    replacement.runtime.as_mut().unwrap().launch_id = "runtime-replica-replacement".into();
    crate::codex_account::mark_runtime_pending(&mut replacement).unwrap();
    let (account, revision) = crate::codex_account::account_for_control_rebind(&replacement)
        .unwrap()
        .expect("resume must reapply the failed account");
    crate::write_session_record(&context, &replacement).unwrap();
    let view = apply_account_binding(
        &mut socket,
        &context,
        &replacement,
        &mut request_id,
        &account,
        revision,
    )
    .await
    .unwrap();
    assert_eq!(view.state, "bound");
    assert_eq!(view.failure_reason, None);
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-replica-replacement")
    );
    let recovered = crate::load_session_record(&context, &record.id).unwrap();
    assert!(crate::codex_account::ensure_input_allowed(&recovered).is_ok());
    assert!(
        !serde_json::to_string(&recovered)
            .unwrap()
            .contains("newer-fixture-token")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn replica_apply_distinguishes_broker_rejection_without_exposing_its_output() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(
        &broker,
        "#!/bin/sh\nprintf '%s\\n' 'private-fixture-error fixture-token' >&2\nexit 9\n",
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&[broker]).unwrap(),
    );
    let (stream, _peer) = UnixStream::pair().unwrap();
    let mut socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
        stream,
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    let mut request_id = 0;
    assert_eq!(
        drive_external_auth_login(&mut socket, &mut request_id, "acct1").await,
        Err("broker_resolve_rejected")
    );
    assert_eq!(
        request_id, 0,
        "broker rejection must not send a provider login"
    );
}
