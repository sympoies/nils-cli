use super::*;
use pretty_assertions::assert_eq;

#[derive(Clone, Copy)]
enum PromptPeer {
    Reject,
    DropAcknowledgement,
    CloseBeforePrompt,
}

async fn prompt_peer(
    listener: tokio::net::UnixListener,
    peer: PromptPeer,
) -> (oneshot::Receiver<()>, tokio::task::JoinHandle<()>) {
    let (ready_tx, ready_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["thread-typed-causes"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        respond(&mut socket, &resume, json!({})).await;
        let usage = receive_json(&mut socket).await;
        assert_eq!(usage["method"], "account/rateLimits/read");
        respond(
            &mut socket,
            &usage,
            json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
        )
        .await;
        let _ = ready_tx.send(());
        match peer {
            PromptPeer::CloseBeforePrompt => {
                let _ = socket
                    .send(tokio_tungstenite::tungstenite::Message::Close(None))
                    .await;
            }
            PromptPeer::Reject => {
                let prompt = receive_json(&mut socket).await;
                assert_eq!(prompt["method"], "turn/start");
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        json!({
                            "id": prompt["id"],
                            "error": { "code": -32000, "message": "provider rejected" }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
            }
            PromptPeer::DropAcknowledgement => {
                let prompt = receive_json(&mut socket).await;
                assert_eq!(prompt["method"], "turn/start");
                drop(socket);
            }
        }
    });
    (ready_rx, task)
}

async fn start_prompt_control(
    id: &str,
    peer: PromptPeer,
) -> (
    tempfile::TempDir,
    CliContext,
    SessionRecord,
    ControlHandle,
    tokio::task::JoinHandle<Result<(), String>>,
    tokio::task::JoinHandle<()>,
) {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket_path = tmp.path().join(format!("{id}.sock"));
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime(id, &socket_path);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-typed-causes").unwrap();
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let (ready_rx, peer_task) = prompt_peer(listener, peer).await;
    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));
    handle.wait_ready(Duration::from_secs(5)).await.unwrap();
    ready_rx.await.unwrap();
    (tmp, context, record, handle, control, peer_task)
}

#[tokio::test]
async fn prompt_readiness_refusal_is_marked_as_not_dispatched() {
    let env_lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let socket_path = tmp.path().join("prompt-readiness.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("prompt-readiness", &socket_path);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-prompt-readiness").unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["thread-prompt-readiness"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        respond(&mut socket, &resume, json!({})).await;
        let usage = receive_json(&mut socket).await;
        assert_eq!(usage["method"], "account/rateLimits/read");
        respond(
            &mut socket,
            &usage,
            json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
        )
        .await;
        tokio::time::timeout(Duration::from_millis(250), receive_json(&mut socket))
            .await
            .ok()
    });
    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));
    handle.wait_ready(Duration::from_secs(5)).await.unwrap();
    crate::codex_account::queue_next_account_with_unbound(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "fixture-next-account",
    )
    .unwrap();

    let error = handle
        .submit_prompt("must remain retryable")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        crate::codex_app_server::prompt_submit::PromptSubmissionFailure::NotDispatched
    ));
    assert!(
        server.await.unwrap().is_none(),
        "turn/start must not be written"
    );
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn websocket_provider_rejection_is_not_confirmed() {
    let (_tmp, _context, _record, handle, control, peer) =
        start_prompt_control("provider-rejection", PromptPeer::Reject).await;
    let error = handle.submit_prompt("rejected prompt").await.unwrap_err();
    assert!(matches!(
        error,
        prompt_submit::PromptSubmissionFailure::NotConfirmed
    ));
    peer.await.unwrap();
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn websocket_disconnect_before_submit_is_not_acknowledgement_unknown() {
    let (_tmp, _context, _record, handle, control, peer) =
        start_prompt_control("write-failure", PromptPeer::CloseBeforePrompt).await;
    peer.await.unwrap();
    let error = handle.submit_prompt("not written").await.unwrap_err();
    assert!(matches!(
        error,
        prompt_submit::PromptSubmissionFailure::NotDispatched
    ));
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn websocket_lost_acknowledgement_is_unknown() {
    let (_tmp, _context, _record, handle, control, peer) =
        start_prompt_control("lost-acknowledgement", PromptPeer::DropAcknowledgement).await;
    let error = handle
        .submit_prompt("sent without an acknowledgement")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        prompt_submit::PromptSubmissionFailure::AcknowledgementUnknown
    ));
    peer.await.unwrap();
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn fresh_thread_start_waits_for_initial_binding_application() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("initial-binding.sock");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("initial-binding", &upstream);
    let runtime_id = record.runtime.as_ref().unwrap().launch_id.clone();
    record.extra.insert(
        "codex_account_binding".to_owned(),
        json!({
            "schema_version": "agent-session.codex-account-binding.v1",
            "selected_account": "fixture-account",
            "selection_source": "explicit",
            "revision": 1,
            "state": "pending",
            "applied_runtime_id": null,
            "updated_at": "2030-01-01T00:00:00Z"
        }),
    );
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    let mut bootstrap = FreshBootstrap::for_runtime(&context, &record);
    assert_eq!(bootstrap, FreshBootstrap::ThreadStart);

    let apply_context = context.clone();
    let id = record.id.clone();
    let apply = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _lock = crate::acquire_session_record_lock(&apply_context, &id).unwrap();
        let mut current = crate::load_session_record(&apply_context, &id).unwrap();
        current.extra.get_mut("codex_account_binding").unwrap()["state"] = json!("bound");
        current.extra.get_mut("codex_account_binding").unwrap()["applied_runtime_id"] =
            json!(runtime_id);
        crate::write_session_record(&apply_context, &current).unwrap();
    });

    let result = cancel_before_tui_mutation_detailed(
        &context,
        &record,
        &mut bootstrap,
        &json!({ "id": 1, "method": "thread/start", "params": {} }),
    )
    .await;
    apply.await.unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn fresh_thread_start_refuses_unresolved_binding_while_create_owns_record_lock() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("locked-initial-binding.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("locked-initial-binding", &upstream);
    record.extra.insert(
        "codex_account_binding".to_owned(),
        json!({
            "schema_version": "agent-session.codex-account-binding.v1",
            "selected_account": "fixture-account",
            "selection_source": "explicit",
            "revision": 1,
            "state": "pending",
            "applied_runtime_id": null,
            "updated_at": "2030-01-01T00:00:00Z"
        }),
    );
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    let _create_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();

    let (upstream_tx, mut upstream_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        let _ = upstream_tx.send(request.clone());
        respond(&mut socket, &request, json!({ "threadId": "thread-a" })).await;
        socket.close(None).await.unwrap();
    });
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let proxy_stream = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match tokio::net::UnixStream::connect(upstream.with_extension("proxy")).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("proxy listener should start");
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({ "id": 1, "method": "thread/start", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(35), receive_json(&mut tui))
        .await
        .expect("thread/start must receive a bounded refusal");

    assert_eq!(response["id"], 1);
    assert_eq!(response["error"]["code"], -32001);
    assert_eq!(
        response["error"]["data"]["reason"],
        "account_authority_unavailable"
    );
    assert_eq!(
        response["error"]["data"]["cause"]["code"],
        "initial-binding-unresolved"
    );
    assert_eq!(
        response["error"]["data"]["cause"]["stage"],
        "initial_binding_wait"
    );
    assert_eq!(
        response["error"]["data"]["cause"]["binding_state"],
        "pending"
    );
    assert_eq!(response["error"]["data"]["retryable"], false);
    assert_eq!(response["error"]["data"]["cause"]["receipt_written"], true);
    let receipt = crate::lifecycle::read(&context, &record.id, 1).unwrap()["records"][0].clone();
    assert_eq!(receipt["result"]["code"], "initial-binding-unresolved");
    assert_eq!(
        receipt["result"]["cause"]["code"],
        response["error"]["data"]["cause"]["code"]
    );
    assert_eq!(receipt["result"]["cause"]["retryable"], false);
    assert!(upstream_rx.try_recv().is_err());

    proxy.abort();
    server.abort();
}

#[test]
fn thread_start_authority_failure_projects_bounded_typed_cause() {
    let rejection = TuiMutationRejection::AccountAuthorityUnavailable {
        operation: "thread_start_cancellation",
        stage: "session_authority_read",
        code: "session-record-unavailable",
        retryable: false,
        binding_state: "pending",
        binding_revision: 8,
        receipt_written: true,
    };
    let Message::Text(text) = tui_busy_response(&json!(7), rejection) else {
        panic!("busy response must be text");
    };
    let response: Value = serde_json::from_str(text.as_str()).unwrap();
    assert_eq!(
        response["error"]["data"]["reason"],
        "account_authority_unavailable"
    );
    assert_eq!(
        response["error"]["data"]["cause"]["operation"],
        "thread_start_cancellation"
    );
    assert_eq!(
        response["error"]["data"]["cause"]["stage"],
        "session_authority_read"
    );
    assert_eq!(
        response["error"]["data"]["cause"]["code"],
        "session-record-unavailable"
    );
    assert_eq!(
        response["error"]["data"]["cause"]["binding_state"],
        "pending"
    );
    assert_eq!(response["error"]["data"]["cause"]["binding_revision"], 8);
    assert_eq!(response["error"]["data"]["cause"]["receipt_written"], true);
    assert_eq!(response["error"]["data"]["retryable"], false);
    let serialized = serde_json::to_string(&response["error"]["data"]).unwrap();
    assert!(serialized.len() <= 512);
    assert!(!serialized.contains("session-id"));
}

#[tokio::test]
async fn busy_manual_cancellation_rejects_only_the_turn_and_keeps_proxy_alive() {
    let env_lock = GlobalStateLock::new();
    let _without_broker = EnvGuard::remove(&env_lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("busy.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("busy-input", &upstream);
    record.extra.insert(
        "codex_account_binding".to_owned(),
        json!({
            "schema_version": "agent-session.codex-account-binding.v1",
            "selected_account": "fixture-account",
            "revision": 6,
            "state": "bound",
            "applied_runtime_id": "runtime-busy-input",
            "updated_at": "2030-01-01T00:00:00Z"
        }),
    );
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["id"], 2);
        respond(&mut socket, &request, json!({ "ok": true })).await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let mut current = crate::load_session_record(&context, &record.id).unwrap();
    current.extra.get_mut("codex_account_binding").unwrap()["revision"] = json!(7);
    crate::write_session_record(&context, &current).unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let rejected = receive_json(&mut tui).await;
    assert_eq!(rejected["id"], 1);
    assert_eq!(rejected["error"]["code"], -32001);
    assert_eq!(
        rejected["error"]["data"]["reason"],
        "manual_cancellation_busy"
    );
    assert_eq!(rejected["error"]["data"]["retryable"], true);
    assert_eq!(rejected["error"]["data"]["next_action"], "retry-request");
    assert_eq!(
        rejected["error"]["data"]["cause"]["operation"],
        "turn_start_cancellation"
    );
    assert_eq!(
        rejected["error"]["data"]["cause"]["stage"],
        "session_record_lock"
    );
    assert_eq!(
        rejected["error"]["data"]["cause"]["code"],
        "session-record-lock-busy"
    );
    assert_eq!(rejected["error"]["data"]["cause"]["binding_state"], "bound");
    assert_eq!(rejected["error"]["data"]["cause"]["binding_revision"], 7);
    assert_eq!(rejected["error"]["data"]["cause"]["receipt_written"], true);
    let receipt = crate::lifecycle::read(&context, &record.id, 1).unwrap()["records"][0].clone();
    assert_eq!(receipt["schema_version"], "agent-session.lifecycle.v1");
    assert_eq!(receipt["caller"]["kind"], "controller");
    assert_eq!(receipt["result"]["code"], "session-record-lock-busy");
    assert_eq!(
        receipt["result"]["message"],
        "lifecycle operation failed; inspect the returned error code"
    );
    assert_eq!(receipt["result"]["proof_step"], "operation-validation");
    assert_eq!(receipt["exit"], Value::Null);
    assert_eq!(
        receipt["session_incarnation"],
        record.runtime.as_ref().unwrap().launch_id
    );
    assert_eq!(receipt["session_generation"], 1);
    assert_eq!(
        receipt["result"]["cause"]["operation"],
        rejected["error"]["data"]["cause"]["operation"]
    );
    assert_eq!(
        receipt["result"]["cause"]["stage"],
        rejected["error"]["data"]["cause"]["stage"]
    );
    assert_eq!(
        receipt["result"]["cause"]["code"],
        rejected["error"]["data"]["cause"]["code"]
    );
    assert_eq!(receipt["result"]["cause"]["retryable"], true);
    assert_eq!(receipt["result"]["cause"]["binding_state"], "bound");
    assert_eq!(receipt["result"]["cause"]["binding_revision"], 7);
    assert!(!receipt.to_string().contains("/repo"));
    drop(lock);
    tui.send(Message::Text(
        json!({ "id": 2, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 2);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}
