//! The service accepts only a lost acknowledgement when the exact runtime stopped.
use super::*;
use crate::codex_app_server::tests::control_fixture;
use pretty_assertions::assert_eq;

#[derive(Clone, Copy)]
enum PeerOutcome {
    RejectPrompt,
    DropAcknowledgement,
    CloseBeforePrompt,
    AcceptPrompt { api_key: bool },
}

async fn receive_json<S>(socket: &mut S) -> Value
where
    S: futures_util::Stream<Item = Result<ClientMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    match socket.next().await.unwrap().unwrap() {
        ClientMessage::Text(text) => serde_json::from_str(&text).unwrap(),
        message => panic!("expected JSON text frame, got {message:?}"),
    }
}

async fn respond<S>(socket: &mut S, request: &Value, result: Value)
where
    S: futures_util::Sink<ClientMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    socket
        .send(ClientMessage::Text(
            json!({ "id": request["id"], "result": result })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

async fn respond_usage<S>(socket: &mut S, api_key: bool)
where
    S: futures_util::Stream<Item = Result<ClientMessage, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<ClientMessage, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let request = receive_json(socket).await;
    assert_eq!(request["method"], "account/rateLimits/read");
    if api_key {
        socket
            .send(ClientMessage::Text(
                json!({
                    "id": request["id"],
                    "error": {
                        "code": -32600,
                        "message": "chatgpt authentication required to read rate limits"
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
    } else {
        respond(
            socket,
            &request,
            json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
        )
        .await;
    }
}

async fn serve_codex_peer(
    listener: tokio::net::UnixListener,
    outcome: PeerOutcome,
    stopped_marker: PathBuf,
) {
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
        json!({ "data": ["thread-codex-outcomes"], "nextCursor": null }),
    )
    .await;
    let resume = receive_json(&mut socket).await;
    assert_eq!(resume["method"], "thread/resume");
    respond(&mut socket, &resume, json!({})).await;
    respond_usage(
        &mut socket,
        matches!(outcome, PeerOutcome::AcceptPrompt { api_key: true }),
    )
    .await;
    match outcome {
        PeerOutcome::AcceptPrompt { api_key } => {
            respond_usage(&mut socket, api_key).await;
            let prompt = receive_json(&mut socket).await;
            assert_eq!(prompt["method"], "turn/start");
            assert_eq!(prompt["params"]["threadId"], "thread-codex-outcomes");
            respond(
                &mut socket,
                &prompt,
                json!({ "turn": { "id": "turn-fixture", "status": "inProgress" } }),
            )
            .await;
            // Keep the peer connected until the control handle is dropped.
            assert!(!matches!(socket.next().await, Some(Ok(frame)) if !frame.is_close()));
        }
        PeerOutcome::CloseBeforePrompt => {
            fs::write(stopped_marker, "").unwrap();
            let _ = socket.send(ClientMessage::Close(None)).await;
        }
        PeerOutcome::RejectPrompt => {
            let prompt = receive_json(&mut socket).await;
            assert_eq!(prompt["method"], "turn/start");
            fs::write(stopped_marker, "").unwrap();
            socket
                .send(ClientMessage::Text(
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
        PeerOutcome::DropAcknowledgement => {
            let prompt = receive_json(&mut socket).await;
            assert_eq!(prompt["method"], "turn/start");
            fs::write(stopped_marker, "").unwrap();
            drop(socket);
        }
    }
}

async fn fixture(
    root: &Path,
    id: &str,
    outcome: PeerOutcome,
) -> (
    nils_test_support::tempdir::ScopedTempDir,
    Arc<ServeState>,
    String,
    tokio::task::JoinHandle<Result<(), String>>,
    tokio::task::JoinHandle<()>,
) {
    let launch = seed_codex_app_server_session(root, id);
    let (socket_dir, socket_path) = nils_test_support::short_socket_path(format!("{id}.sock"));
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: root.to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, id).unwrap();
    let runtime_extra = &mut record.runtime.as_mut().unwrap().extra;
    runtime_extra.insert(
        codex_app_server::SOCKET_KEY.to_string(),
        json!(crate::display_path(&socket_path)),
    );
    runtime_extra.insert(
        codex_app_server::PROXY_KEY.to_string(),
        json!(crate::display_path(&socket_path.with_extension("proxy"))),
    );
    record.extra.insert(
        "delete_tmux_identity".into(),
        json!({ "launch_id": launch, "session_id": "$96", "pane_id": "%96", "pane_pid": 96 }),
    );
    crate::write_session_record(&context, &record).unwrap();
    codex_app_server::replace_thread_binding(&record, "thread-codex-outcomes").unwrap();

    let stopped_marker = root.join("stopped");
    let peer = tokio::spawn(serve_codex_peer(listener, outcome, stopped_marker.clone()));
    let tmux = empty_composer_tmux(root);
    let script = fs::read_to_string(&tmux).unwrap().replace(
        "has-session) exit 0 ;;",
        &format!(
            "has-session) if [ -f '{}' ]; then printf '%s\\n' \"can't find session: $96\" >&2; exit 1; else exit 0; fi ;;",
            stopped_marker.display()
        ),
    );
    fs::write(&tmux, script).unwrap();
    let st = state(root, Some(TOKEN), tmux);
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let (handle, commands, ready) = codex_app_server::starting_control_channel();
    let control = tokio::spawn(codex_app_server::run_control(
        st.context.clone(),
        record,
        commands,
        ready,
    ));
    handle.wait_ready(Duration::from_secs(5)).await.unwrap();
    st.codex_controls.lock().unwrap().insert(
        id.to_string(),
        CodexControlEntry {
            launch_id: launch.clone(),
            handle,
        },
    );
    (socket_dir, st, launch, control, peer)
}

async fn submit(st: Arc<ServeState>, id: &str, launch: &str) -> (StatusCode, Value) {
    call(
        router(st),
        post_json(
            &format!("/sessions/{id}/prompt/v2"),
            Some(TOKEN),
            json!({
                "text": "inspect provider acknowledgement",
                "expected_session_incarnation": launch,
                "request_id": "protocol-attempt"
            }),
        ),
    )
    .await
}

async fn assert_prompt_admitted_with_usage(root: &Path, id: &str, api_key: bool) {
    let (_socket_dir, st, launch, control, peer) =
        fixture(root, id, PeerOutcome::AcceptPrompt { api_key }).await;
    let handle = st.codex_controls.lock().unwrap()[id].handle.clone();
    let usage = handle.usage().await;
    if api_key {
        assert!(usage.is_err(), "API-key usage must be unavailable");
    } else {
        assert!(usage.unwrap().authoritative);
    }

    let (status, body) = submit(st.clone(), id, &launch).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(body["data"]["receipt"]["request_id"], "protocol-attempt");
    assert!(!control.is_finished(), "control must stay connected");

    st.codex_controls.lock().unwrap().remove(id).unwrap();
    drop(handle);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), control)
            .await
            .unwrap()
            .unwrap(),
        Ok(())
    );
    peer.await.unwrap();
}

#[test]
fn already_attached_api_key_control_admits_keyed_prompt_without_usage() {
    control_fixture::run(async |root| {
        assert_prompt_admitted_with_usage(root, "api-key-control", true).await
    });
}

#[test]
fn chatgpt_control_reads_usage_and_admits_keyed_prompt() {
    control_fixture::run(async |root| {
        assert_prompt_admitted_with_usage(root, "chatgpt-control", false).await
    });
}

#[tokio::test]
async fn first_keyed_api_key_prompt_survives_delayed_thread_start() {
    let id = "delayed-api-key-start";
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_codex_app_server_session(tmp.path(), id);
    let (_socket_dir, upstream) = nils_test_support::short_socket_path("delayed-start.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let proxy_path = upstream.with_extension("proxy");
    let (tmux, terminal_enter) = empty_composer_tmux_marking_enter(tmp.path());
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let mut record = load_session_record(&st.context, id).unwrap();
    let runtime = record.runtime.as_mut().unwrap();
    runtime.extra.insert(
        codex_app_server::SOCKET_KEY.into(),
        json!(crate::display_path(&upstream)),
    );
    runtime.extra.insert(
        codex_app_server::PROXY_KEY.into(),
        json!(crate::display_path(&proxy_path)),
    );
    crate::write_session_record(&st.context, &record).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let marker = codex_app_server::thread_attached_path(&record)
        .unwrap()
        .to_owned();
    assert!(!marker.exists());

    let proxy_context = st.context.clone();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: id.into(),
        upstream,
        listen: proxy_path.clone(),
    };
    let proxy = tokio::task::spawn_blocking(move || {
        codex_app_server::run_proxy(&proxy_context, proxy_args)
    });
    let (startup_seen, startup_request) = tokio::sync::oneshot::channel();
    let (startup_release, release) = tokio::sync::oneshot::channel();
    let (control_initialized, control_initialized_rx) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut tui_peer = tokio_tungstenite::accept_async(stream).await.unwrap();
        let start = receive_json(&mut tui_peer).await;
        assert_eq!(start["method"], "thread/start");
        startup_seen.send(()).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let mut control_peer = tokio_tungstenite::accept_async(stream).await.unwrap();
        let init = receive_json(&mut control_peer).await;
        respond(&mut control_peer, &init, json!({})).await;
        assert_eq!(
            receive_json(&mut control_peer).await["method"],
            "initialized"
        );
        control_initialized.send(()).unwrap();
        let mut release = std::pin::pin!(release);
        let mut thread_started = false;
        let mut turn_started = false;
        loop {
            tokio::select! {
                result = &mut release, if !thread_started => {
                    result.unwrap();
                    respond(&mut tui_peer, &start, json!({
                        "thread": {"id": "thread-delayed", "status": {"type": "idle"}}
                    })).await;
                    thread_started = true;
                }
                frame = control_peer.next() => {
                    let Some(Ok(ClientMessage::Text(text))) = frame else { break };
                    let request: Value = serde_json::from_str(&text).unwrap();
                    match request["method"].as_str().unwrap() {
                        "thread/loaded/list" => {
                            let ids = if thread_started { json!(["thread-delayed"]) } else { json!([]) };
                            respond(&mut control_peer, &request, json!({"data": ids})).await;
                        }
                        "thread/resume" => respond(&mut control_peer, &request, json!({})).await,
                        "account/rateLimits/read" => {
                            control_peer.send(ClientMessage::Text(json!({
                                "id": request["id"],
                                "error": {"code": -32600, "message": "chatgpt authentication required to read rate limits"}
                            }).to_string().into())).await.unwrap();
                        }
                        "turn/start" => {
                            assert!(thread_started);
                            assert!(!turn_started, "only one turn may be submitted");
                            assert_eq!(request["params"]["threadId"], "thread-delayed");
                            respond(&mut control_peer, &request, json!({"turn": {"id":"turn-delayed"}})).await;
                            turn_started = true;
                        }
                        method => panic!("unexpected control request {method}"),
                    }
                }
            }
        }
        // Keep the proxy peer alive until the synthetic TUI closes the connection.
        let _ = tui_peer.next().await;
        turn_started
    });

    let tui_stream = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(stream) = tokio::net::UnixStream::connect(&proxy_path).await {
                break stream;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", tui_stream)
        .await
        .unwrap();
    tui.send(ClientMessage::Text(
        json!({
            "id": 1, "method": "thread/start", "params": {}
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    startup_request.await.unwrap();

    let (handle, commands, ready) = codex_app_server::starting_control_channel();
    let control = tokio::spawn(codex_app_server::run_control(
        st.context.clone(),
        record,
        commands,
        ready,
    ));
    st.codex_controls.lock().unwrap().insert(
        id.into(),
        CodexControlEntry {
            launch_id: launch.clone(),
            handle,
        },
    );
    control_initialized_rx.await.unwrap();
    let prompt_state = st.clone();
    let prompt = tokio::spawn(async move { submit(prompt_state, id, &launch).await });
    // The proxy cannot attach until the peer completes the TUI request.
    tokio::time::sleep(Duration::from_secs(11)).await;
    let _ = startup_release.send(());
    let (status, body) = prompt.await.unwrap();
    let ready_after_prompt = !control.is_finished();
    let bound = marker.is_file();

    st.codex_controls.lock().unwrap().remove(id);
    tui.send(ClientMessage::Close(None)).await.unwrap();
    let control_result = control.await.unwrap();
    let turn_started = peer.await.unwrap();
    assert_eq!(proxy.await.unwrap(), 0);

    assert_eq!(
        status,
        StatusCode::OK,
        "{body}; control result: {control_result:?}"
    );
    assert_eq!(body["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(body["data"]["receipt"]["request_id"], "protocol-attempt");
    assert!(bound, "proxy must write the attachment");
    assert!(
        !terminal_enter.exists(),
        "structured submission must not use the terminal"
    );
    assert!(turn_started, "the first keyed prompt must start a turn");
    assert!(
        ready_after_prompt,
        "the original control must remain connected"
    );
    assert_eq!(control_result, Ok(()));
}

#[test]
fn provider_rejection_with_stopped_runtime_remains_unknown() {
    control_fixture::run(async |root| {
        let (_socket_dir, st, launch, control, peer) =
            fixture(root, "provider-rejected-exit", PeerOutcome::RejectPrompt).await;
        let (status, body) = submit(st, "provider-rejected-exit", &launch).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
        assert_eq!(body["error"]["details"]["receipt"]["outcome"], "unknown");
        peer.await.unwrap();
        control.abort();
        let _ = control.await;
    });
}

#[test]
fn lost_acknowledgement_with_stopped_runtime_is_submitted() {
    control_fixture::run(async |root| {
        let (_socket_dir, st, launch, control, peer) =
            fixture(root, "lost-ack-exit", PeerOutcome::DropAcknowledgement).await;
        let (status, body) = submit(st, "lost-ack-exit", &launch).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["receipt"]["outcome"], "submitted");
        peer.await.unwrap();
        control.abort();
        let _ = control.await;
    });
}

#[test]
fn closed_websocket_with_stopped_runtime_remains_unknown() {
    control_fixture::run(async |root| {
        let (_socket_dir, st, launch, control, peer) = fixture(
            root,
            "closed-websocket-exit",
            PeerOutcome::CloseBeforePrompt,
        )
        .await;
        peer.await.unwrap();
        let (status, body) = submit(st, "closed-websocket-exit", &launch).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
        control.abort();
        let _ = control.await;
    });
}
