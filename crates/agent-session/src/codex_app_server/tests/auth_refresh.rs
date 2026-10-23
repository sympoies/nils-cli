use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn external_auth_refresh_rebinds_durable_state_without_serializing_token() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let calls = tmp.path().join("calls");
    fs::write(
        &broker,
        r#"#!/bin/sh
calls=$1
shift
printf '%s\n' "$*" >> "$calls"
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"refreshed-fixture-token","chatgpt_account_id":"workspace-refreshed","plan":"team"}'
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![
        broker.to_string_lossy().into_owned(),
        calls.to_string_lossy().into_owned(),
    ])
    .unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("refresh-success", &tmp.path().join("server.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-refresh-success",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();

    let mut sink = RecordingMessageSink::default();
    assert!(
        respond_to_external_auth_refresh(
            &mut sink,
            &json!({
                "id": "refresh-1",
                "method": "account/chatgptAuthTokens/refresh",
                "params": {
                    "reason": "unauthorized",
                    "previousAccountId": "workspace-old"
                }
            }),
            Some((&context, &record, "acct1", "fixture-connection")),
        )
        .await
        .unwrap()
    );
    let incident = crate::auth_incident::view(&context, &record).unwrap();
    assert_eq!(
        incident.source,
        crate::auth_incident::AuthSource::CodexExternalRefresh
    );
    assert_eq!(
        incident.recovery_result.as_deref(),
        Some("credentials_refreshed")
    );
    assert_eq!(incident.status, "recovered");
    let response: Value = match &sink.messages[0] {
        Message::Text(text) => serde_json::from_str(text).unwrap(),
        message => panic!("unexpected refresh response: {message:?}"),
    };
    assert_eq!(response["id"], "refresh-1");
    assert_eq!(response["result"]["accessToken"], "refreshed-fixture-token");
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.state, "bound");
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-refresh-success")
    );
    let session_json =
        fs::read_to_string(crate::session_dir(&context, &record.id).join("session.json")).unwrap();
    assert!(!session_json.contains("refreshed-fixture-token"));
    assert_eq!(
        fs::read_to_string(&calls).unwrap().trim(),
        "resolve --account acct1 --force-refresh --format json"
    );
    let previous_incident = incident.incident_id;
    assert!(respond_to_external_auth_refresh(&mut sink, &json!({"id":"refresh-1", "method":"account/chatgptAuthTokens/refresh", "params":{"reason":"unauthorized"}}), Some((&context, &record, "acct1", "fixture-connection"))).await.is_err());
    assert_eq!(sink.messages.len(), 1);
    assert_eq!(
        crate::auth_incident::view(&context, &record)
            .unwrap()
            .incident_id,
        previous_incident
    );
    // A broken reporting store must not prevent the credential response.
    fs::write(
        crate::session_dir(&context, &record.id).join("auth-incidents.json"),
        b"invalid",
    )
    .unwrap();
    assert!(respond_to_external_auth_refresh(&mut sink, &json!({"id":"refresh-2", "method":"account/chatgptAuthTokens/refresh", "params":{"reason":"unauthorized"}}), Some((&context, &record, "acct1", "fixture-connection"))).await.unwrap());
    assert_eq!(sink.messages.len(), 2);
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 2);
    assert_eq!(
        crate::auth_incident::detection_health(&context, &record).as_deref(),
        Some("degraded_store_unavailable")
    );
}

#[tokio::test]
async fn external_auth_refresh_failure_restores_bound_binding_for_retry() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, "#!/bin/sh\nexit 9\n").unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("refresh-failure", &tmp.path().join("server.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-refresh-failure",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();

    let error = respond_to_external_auth_refresh(
        &mut RecordingMessageSink::default(),
        &json!({
            "id": 19,
            "method": "account/chatgptAuthTokens/refresh",
            "params": { "reason": "unauthorized" }
        }),
        Some((&context, &record, "acct1", "fixture-connection")),
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("Codex account refresh failed:"));
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.state, "bound");
    assert_eq!(view.failure_reason, None);
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-refresh-failure")
    );
    assert_eq!(
        crate::auth_incident::view(&context, &record)
            .unwrap()
            .recovery_result
            .as_deref(),
        Some("refresh_failed")
    );
    assert!(crate::codex_account::ensure_input_allowed(&persisted).is_ok());

    let backoff = respond_to_external_auth_refresh(
        &mut RecordingMessageSink::default(),
        &json!({"id":20,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}),
        Some((&context, &record, "acct1", "fixture-connection")),
    ).await.unwrap_err();
    assert!(backoff.contains("retry-backoff"), "{backoff}");
    let receipt_path = crate::session_dir(&context, &record.id).join("codex-auth-recovery.json");
    let mut receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    receipt["operations"][0]["started_at_epoch_ms"] =
        json!(Timestamp::now().as_millisecond() - 2_000);
    fs::write(receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();

    fs::write(
        &broker,
        r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"retry-fixture-token","chatgpt_account_id":"workspace-retry"}'
"#,
    )
    .unwrap();
    let mut retry_sink = RecordingMessageSink::default();
    assert!(
        respond_to_external_auth_refresh(
            &mut retry_sink,
            &json!({
                "id": 20,
                "method": "account/chatgptAuthTokens/refresh",
                "params": { "reason": "unauthorized" }
            }),
            Some((&context, &record, "acct1", "fixture-connection")),
        )
        .await
        .unwrap()
    );
    let retried = crate::load_session_record(&context, &record.id).unwrap();
    let retry_view = crate::codex_account::view_for_record(&retried);
    assert_eq!(retry_view.state, "bound");
    assert_eq!(retry_view.revision, 3);
    assert_eq!(retry_sink.messages.len(), 1);
}

#[tokio::test]
async fn external_auth_refresh_send_failure_restores_bound_binding_for_retry() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(
        &broker,
        r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"send-failure-fixture-token","chatgpt_account_id":"workspace-send-failure"}'
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("refresh-send-failure", &tmp.path().join("server.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-refresh-send-failure",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();

    let error = respond_to_external_auth_refresh(
        &mut FailingMessageSink,
        &json!({
            "id": 21,
            "method": "account/chatgptAuthTokens/refresh",
            "params": { "reason": "unauthorized" }
        }),
        Some((&context, &record, "acct1", "fixture-connection")),
    )
    .await
    .unwrap_err();

    assert!(error.starts_with("Codex app-server write failed:"));
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.state, "bound");
    assert_eq!(view.revision, 2);
    assert_eq!(view.selected_account.as_deref(), Some("acct1"));
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-refresh-send-failure")
    );
    assert!(crate::codex_account::ensure_input_allowed(&persisted).is_ok());
}

fn recovery_fixture(tmp: &tempfile::TempDir) -> (CliContext, SessionRecord) {
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("recovery-budget", &tmp.path().join("server.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    record.provider_resume = Some(ProviderResume {
        provider: "codex".into(),
        session_id: "fixture-thread".into(),
        captured_at: "2030-01-01T00:00:00Z".into(),
        capture_method: PROVIDER_RESUME_CAPTURE_METHOD.into(),
        resume_args: vec!["resume".into(), "fixture-thread".into()],
        extra: BTreeMap::new(),
    });
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-recovery-budget",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();
    (context, record)
}

#[tokio::test]
async fn external_auth_refresh_bounds_consecutive_faults_and_keeps_identity() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let calls = tmp.path().join("calls");
    fs::write(&broker, format!("#!/bin/sh\necho called >> '{}'\nprintf '%s\\n' '{{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"acct1\",\"access_token\":\"fixture-secret\",\"chatgpt_account_id\":\"fixture-workspace\"}}'\n", calls.display())).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker]).unwrap(),
    );
    let (context, record) = recovery_fixture(&tmp);
    let before = crate::load_session_record(&context, &record.id).unwrap();
    #[cfg(target_os = "linux")]
    crate::coordination::auth_effects::test_support::seed(&context, &record);
    // Real broker subprocess -> controller -> provider websocket, with no
    // prompt or turn-start message on the transport.
    let (client, server) = tokio::net::UnixStream::pair().unwrap();
    let peer = tokio::spawn(async move {
        let mut socket = tokio_tungstenite::accept_async(server).await.unwrap();
        let mut messages = Vec::new();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            messages.push(serde_json::from_str::<Value>(&text).unwrap());
        }
        messages
    });
    let (mut sink, _) = tokio_tungstenite::client_async("ws://localhost", client)
        .await
        .unwrap();
    for id in 1..=3 {
        assert!(respond_to_external_auth_refresh(&mut sink, &json!({"id":id,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", "fixture-connection"))).await.unwrap());
    }
    let error = respond_to_external_auth_refresh(&mut sink, &json!({"id":4,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", "fixture-connection"))).await.unwrap_err();
    assert!(error.contains("retry-budget-exhausted"), "{error}");
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 3);
    // Expiring the rate window admits a new official request, but never
    // discards identities needed to refuse replay of an older response.
    let path = crate::session_dir(&context, &record.id).join("codex-auth-recovery.json");
    let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for operation in stored["operations"].as_array_mut().unwrap() {
        operation["started_at_epoch_ms"] = json!(Timestamp::now().as_millisecond() - 61_000);
    }
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    respond_to_external_auth_refresh(&mut sink, &json!({"id":4,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", "fixture-connection"))).await.unwrap();
    assert!(respond_to_external_auth_refresh(&mut sink, &json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", "fixture-connection"))).await.unwrap_err().contains("duplicate-request"));
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 4);
    sink.close(None).await.unwrap();
    let received = peer.await.unwrap();
    assert_eq!(received.len(), 4);
    for (index, message) in received.iter().enumerate() {
        assert_eq!(message["id"], index + 1);
        assert_eq!(message["result"]["accessToken"], "fixture-secret");
        assert!(message.get("method").is_none());
    }
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        serde_json::to_value(&current.runtime).unwrap(),
        serde_json::to_value(&before.runtime).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&current.provider_resume).unwrap(),
        serde_json::to_value(&before.provider_resume).unwrap()
    );
    assert_eq!(
        crate::codex_account::selected_account(&current).as_deref(),
        Some("acct1")
    );
    let receipt = fs::read_to_string(
        crate::session_dir(&context, &record.id).join("codex-auth-recovery.json"),
    )
    .unwrap();
    assert!(!receipt.contains("fixture-secret"));
    assert!(!receipt.contains("sensitive-canary"));
    let receipt: Value = serde_json::from_str(&receipt).unwrap();
    assert_eq!(receipt["operations"][0]["state"], "credentials_supplied");
    assert_eq!(receipt["operations"][0]["replayed"], false);
    assert_eq!(
        receipt["operations"][0]["provider_session_id"],
        "fixture-thread"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(receipt["operations"][0]["unknown_operation_count"], 2);
}

#[tokio::test]
async fn external_auth_refresh_rejects_stale_runtime_before_broker_or_incident() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        "[\"/nonexistent-synthetic-broker\"]",
    );
    let (context, record) = recovery_fixture(&tmp);
    let mut current = crate::load_session_record(&context, &record.id).unwrap();
    current.runtime.as_mut().unwrap().generation += 1;
    crate::write_session_record(&context, &current).unwrap();
    let error = respond_to_external_auth_refresh(&mut RecordingMessageSink::default(), &json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", "fixture-connection"))).await.unwrap_err();
    assert!(error.contains("superseded"), "{error}");
    assert!(crate::auth_incident::view(&context, &current).is_none());
    assert_eq!(
        crate::codex_account::view_for_record(
            &crate::load_session_record(&context, &record.id).unwrap()
        )
        .revision,
        1
    );
}

#[tokio::test]
async fn external_auth_refresh_crash_receipts_never_repeat_broker_or_response() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let calls = tmp.path().join("calls");
    fs::write(&broker, format!("#!/bin/sh\necho called >> '{}'\nprintf '%s\\n' '{{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"acct1\",\"access_token\":\"crash-fixture-token\",\"chatgpt_account_id\":\"fixture-workspace\"}}'\n", calls.display())).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker]).unwrap(),
    );
    let (context, record) = recovery_fixture(&tmp);
    let mut sink = RecordingMessageSink::default();
    let request = json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}});
    respond_to_external_auth_refresh(
        &mut sink,
        &request,
        Some((&context, &record, "acct1", "fixture-connection")),
    )
    .await
    .unwrap();
    let path = crate::session_dir(&context, &record.id).join("codex-auth-recovery.json");
    for state in ["resolving", "response_unknown"] {
        let mut receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        receipt["operations"][0]["state"] = json!(state);
        fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        // Reload all durable state as a replacement controller would.
        let reloaded = crate::load_session_record(&context, &record.id).unwrap();
        assert!(
            respond_to_external_auth_refresh(
                &mut sink,
                &request,
                Some((&context, &reloaded, "acct1", "fixture-connection"))
            )
            .await
            .unwrap_err()
            .contains("duplicate-request")
        );
        assert!(
            respond_to_external_auth_refresh(
                &mut sink,
                &request,
                Some((&context, &reloaded, "acct1", "replacement-connection"))
            )
            .await
            .unwrap_err()
            .contains("previous-outcome-unknown")
        );
        let new_request = json!({"id":2,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}});
        assert!(
            respond_to_external_auth_refresh(
                &mut sink,
                &new_request,
                Some((&context, &reloaded, "acct1", "fixture-connection"))
            )
            .await
            .unwrap_err()
            .contains("previous-outcome-unknown")
        );
    }
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 1);
    assert_eq!(sink.messages.len(), 1);
}

#[tokio::test]
async fn external_auth_refresh_stalled_response_has_bounded_unknown_outcome() {
    struct StalledSink;
    impl futures_util::Sink<Message> for StalledSink {
        type Error = tokio_tungstenite::tungstenite::Error;
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }
        fn start_send(self: std::pin::Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            unreachable!()
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"deadline-fixture-token","chatgpt_account_id":"fixture-workspace"}'
"#).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker]).unwrap(),
    );
    let (context, record) = recovery_fixture(&tmp);
    let error = tokio::time::timeout(Duration::from_secs(2), respond_to_external_auth_refresh(&mut StalledSink,
        &json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", "fixture-connection"))))
        .await.expect("response deadline must release the controller").unwrap_err();
    assert!(error.contains("response-outcome-unknown"), "{error}");
    let receipt: Value = serde_json::from_slice(
        &fs::read(crate::session_dir(&context, &record.id).join("codex-auth-recovery.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["operations"][0]["state"], "response_unknown");
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::codex_account::view_for_record(&current).state,
        "bound"
    );
}

#[tokio::test]
async fn external_auth_refresh_fences_conversation_changed_during_broker_call() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, r#"#!/bin/sh
sed 's/fixture-thread/replacement-thread/g' "$1" > "$1.tmp"
mv "$1.tmp" "$1"
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"fenced-fixture-token","chatgpt_account_id":"fixture-workspace"}'
"#).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _configured = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![&broker]).unwrap(),
    );
    let (context, record) = recovery_fixture(&tmp);
    let argv = vec![
        broker,
        crate::session_dir(&context, &record.id).join("session.json"),
    ];
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&argv).unwrap(),
    );
    let mut sink = RecordingMessageSink::default();
    let error = respond_to_external_auth_refresh(&mut sink, &json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", "fixture-connection"))).await.unwrap_err();
    assert!(
        error.contains("conversation-or-binding-superseded"),
        "{error}"
    );
    assert!(sink.messages.is_empty());
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::codex_account::view_for_record(&current).state,
        "bound"
    );
    assert_eq!(
        current.provider_resume.unwrap().session_id,
        "replacement-thread"
    );
}

#[tokio::test]
async fn external_auth_refresh_allows_request_id_reuse_after_reconnect() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"reconnect-fixture-token","chatgpt_account_id":"fixture-workspace"}'
"#).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker]).unwrap(),
    );
    let (context, record) = recovery_fixture(&tmp);
    for connection in ["first-connection", "second-connection"] {
        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            let mut socket = tokio_tungstenite::accept_async(server).await.unwrap();
            socket.next().await.unwrap().unwrap()
        });
        let (mut socket, _) = tokio_tungstenite::client_async("ws://localhost", client)
            .await
            .unwrap();
        respond_to_external_auth_refresh(&mut socket, &json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}), Some((&context,&record,"acct1", connection))).await.expect("a new connection can reuse a JSON-RPC request id");
        assert!(matches!(peer.await.unwrap(), Message::Text(_)));
    }
}

#[path = "auth_refresh_retention.rs"]
mod retention;
