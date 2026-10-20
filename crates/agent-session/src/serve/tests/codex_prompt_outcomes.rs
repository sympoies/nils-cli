//! The service accepts only a lost acknowledgement when the exact runtime stopped.
use super::*;
use pretty_assertions::assert_eq;

#[derive(Clone, Copy)]
enum PeerOutcome {
    RejectPrompt,
    DropAcknowledgement,
    CloseBeforePrompt,
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
    let usage = receive_json(&mut socket).await;
    assert_eq!(usage["method"], "account/rateLimits/read");
    respond(
        &mut socket,
        &usage,
        json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
    )
    .await;
    match outcome {
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
    id: &str,
    outcome: PeerOutcome,
) -> (
    tempfile::TempDir,
    Arc<ServeState>,
    String,
    tokio::task::JoinHandle<Result<(), String>>,
    tokio::task::JoinHandle<()>,
) {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_codex_app_server_session(tmp.path(), id);
    let socket_path = tmp.path().join(format!("{id}.sock"));
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, id).unwrap();
    record.extra.insert(
        "delete_tmux_identity".into(),
        json!({ "launch_id": launch, "session_id": "$96", "pane_id": "%96", "pane_pid": 96 }),
    );
    crate::write_session_record(&context, &record).unwrap();
    codex_app_server::replace_thread_binding(&record, "thread-codex-outcomes").unwrap();

    let stopped_marker = tmp.path().join("stopped");
    let peer = tokio::spawn(serve_codex_peer(listener, outcome, stopped_marker.clone()));
    let tmux = empty_composer_tmux(tmp.path());
    let script = fs::read_to_string(&tmux).unwrap().replace(
        "has-session) exit 0 ;;",
        &format!(
            "has-session) if [ -f '{}' ]; then printf '%s\\n' \"can't find session: $96\" >&2; exit 1; else exit 0; fi ;;",
            stopped_marker.display()
        ),
    );
    fs::write(&tmux, script).unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);
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
    (tmp, st, launch, control, peer)
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

#[tokio::test]
async fn provider_rejection_with_stopped_runtime_remains_unknown() {
    let (tmp, st, launch, control, peer) =
        fixture("provider-rejected-exit", PeerOutcome::RejectPrompt).await;
    let (status, body) = submit(st, "provider-rejected-exit", &launch).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
    assert_eq!(body["error"]["details"]["receipt"]["outcome"], "unknown");
    peer.await.unwrap();
    control.abort();
    let _ = control.await;
    drop(tmp);
}

#[tokio::test]
async fn lost_acknowledgement_with_stopped_runtime_is_submitted() {
    let (tmp, st, launch, control, peer) =
        fixture("lost-ack-exit", PeerOutcome::DropAcknowledgement).await;
    let (status, body) = submit(st, "lost-ack-exit", &launch).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["receipt"]["outcome"], "submitted");
    peer.await.unwrap();
    control.abort();
    let _ = control.await;
    drop(tmp);
}

#[tokio::test]
async fn closed_websocket_with_stopped_runtime_remains_unknown() {
    let (tmp, st, launch, control, peer) =
        fixture("closed-websocket-exit", PeerOutcome::CloseBeforePrompt).await;
    peer.await.unwrap();
    let (status, body) = submit(st, "closed-websocket-exit", &launch).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
    control.abort();
    let _ = control.await;
    drop(tmp);
}
