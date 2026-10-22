use super::super::tests::record_with_runtime;
use super::super::*;
use super::{HARD_MESSAGE_LIMIT_BYTES, MAX_FORWARDED_MESSAGE_BYTES, Oversized, contain_over};
use pretty_assertions::assert_eq;
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;

/// The `plugin/list` response that motivated these tests was 16.8 MB and grows
/// with a remote catalog, so the relayed size sits just above the old 16 MiB cap.
const ABOVE_OLD_CAP_BYTES: usize = 17 * 1024 * 1024;
/// Far beyond any legitimate response, so the proxy refuses to forward it.
const ABOVE_FORWARD_LIMIT_BYTES: usize = 65 * 1024 * 1024;

const _: () = assert!(ABOVE_OLD_CAP_BYTES < MAX_FORWARDED_MESSAGE_BYTES);
const _: () = assert!(MAX_FORWARDED_MESSAGE_BYTES < ABOVE_FORWARD_LIMIT_BYTES);
const _: () = assert!(ABOVE_FORWARD_LIMIT_BYTES < HARD_MESSAGE_LIMIT_BYTES);

fn text(value: &str) -> Message {
    Message::Text(value.to_string().into())
}

#[test]
fn containment_classifies_by_json_rpc_shape_and_ignores_messages_within_the_limit() {
    let response = text(r#"{"result":{"k":"vvvvvvvv"},"id":7}"#);
    let request = text(r#"{"id":"srv","method":"m","params":{"k":"vvvvvvvv"}}"#);
    let notification = text(r#"{"method":"m","params":{"k":"vvvvvvvv"}}"#);
    let invalid_id = text(r#"{"id":{"x":1},"result":{"k":"vvvvvvvv"}}"#);
    for message in [&response, &request, &notification, &invalid_id] {
        assert_eq!(contain_over(message, 4096).unwrap(), None);
    }
    assert_eq!(
        contain_over(&Message::Pong(vec![0; 64].into()), 1).unwrap(),
        None
    );

    let Some(Oversized::ToTui(Message::Text(reply))) = contain_over(&response, 16).unwrap() else {
        panic!("a response is answered towards the TUI");
    };
    let reply: Value = serde_json::from_str(reply.as_str()).unwrap();
    assert_eq!(reply["id"], json!(7));
    assert_eq!(reply["error"]["data"]["limit_bytes"], json!(16));
    assert_eq!(reply["error"]["data"]["size_bytes"], json!(response.len()));
    assert!(
        !reply.to_string().contains("vvvv"),
        "payload must not be echoed"
    );

    let Some(Oversized::ToUpstream(Message::Text(reply))) = contain_over(&request, 16).unwrap()
    else {
        panic!("a server request is rejected towards the server");
    };
    assert_eq!(
        serde_json::from_str::<Value>(reply.as_str()).unwrap()["id"],
        "srv"
    );
    assert_eq!(
        contain_over(&notification, 16).unwrap(),
        Some(Oversized::Drop)
    );
    assert_eq!(
        contain_over(&invalid_id, 16).unwrap(),
        Some(Oversized::Drop)
    );
    assert_eq!(
        contain_over(&text("not json at all, but long"), 16).unwrap_err(),
        "proxy observed malformed oversized JSON"
    );
}

struct Fixture {
    _tmp: tempfile::TempDir,
    context: CliContext,
    upstream: PathBuf,
    listener: Option<UnixListener>,
    id: String,
}

fn fixture(id: &str) -> Fixture {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("upstream.sock");
    let listener = std::os::unix::net::UnixListener::bind(&upstream).unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = UnixListener::from_std(listener).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime(id, &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let id = record.id.clone();
    Fixture {
        _tmp: tmp,
        context,
        upstream,
        listener: Some(listener),
        id,
    }
}

impl Fixture {
    fn args(&self) -> crate::cli::CodexAppServerProxyArgs {
        crate::cli::CodexAppServerProxyArgs {
            id: self.id.clone(),
            upstream: self.upstream.clone(),
            listen: self.upstream.with_extension("proxy"),
        }
    }

    fn take_listener(&mut self) -> UnixListener {
        self.listener.take().unwrap()
    }
}

fn spawn_proxy(fixture: &Fixture) -> JoinHandle<Result<(), String>> {
    let context = fixture.context.clone();
    let args = fixture.args();
    tokio::spawn(async move { run_proxy_session(context, args).await })
}

async fn connect_tui(fixture: &Fixture) -> WebSocketStream<UnixStream> {
    let stream = connect_socket(&fixture.upstream.with_extension("proxy"))
        .await
        .unwrap();
    // The TUI side must be able to hold the oversized fixtures so that only
    // the proxy's own limits decide the outcome.
    let config = WebSocketConfig::default()
        .max_message_size(Some(512 * 1024 * 1024))
        .max_frame_size(Some(512 * 1024 * 1024));
    tokio_tungstenite::client_async_with_config("ws://localhost", stream, Some(config))
        .await
        .unwrap()
        .0
}

async fn accept_server(listener: &UnixListener) -> WebSocketStream<UnixStream> {
    let (stream, _) = listener.accept().await.unwrap();
    tokio_tungstenite::accept_async(stream).await.unwrap()
}

async fn next_json(socket: &mut WebSocketStream<UnixStream>) -> Value {
    let message = socket.next().await.unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

async fn send_json_value(socket: &mut WebSocketStream<UnixStream>, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

/// A response whose `result` is a string of `bytes` bytes in total.
fn padded_text(prefix: &str, bytes: usize) -> String {
    let suffix = "\"}}";
    let padding = bytes - prefix.len() - suffix.len();
    format!("{prefix}{}{suffix}", "x".repeat(padding))
}

async fn finish(tui: &mut WebSocketStream<UnixStream>, proxy: JoinHandle<Result<(), String>>) {
    tui.close(None).await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn proxy_relays_a_server_response_above_the_previous_16_mib_cap() {
    let mut fixture = fixture("oversize-relay");
    let proxy = spawn_proxy(&fixture);
    let listener = fixture.take_listener();
    let server = tokio::spawn(async move {
        let mut socket = accept_server(&listener).await;
        let first = next_json(&mut socket).await;
        assert_eq!(first["method"], "plugin/list");
        let text = padded_text(
            &format!(r#"{{"id":{},"result":{{"catalog":""#, first["id"]),
            ABOVE_OLD_CAP_BYTES,
        );
        socket.send(Message::Text(text.into())).await.unwrap();
        let second = next_json(&mut socket).await;
        send_json_value(&mut socket, json!({ "id": second["id"], "result": {} })).await;
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
    });
    let mut tui = connect_tui(&fixture).await;
    send_json_value(
        &mut tui,
        json!({ "id": 1, "method": "plugin/list", "params": {} }),
    )
    .await;
    let large = tui.next().await.unwrap().unwrap().into_text().unwrap();
    assert_eq!(large.len(), ABOVE_OLD_CAP_BYTES);
    assert_eq!(
        serde_json::from_str::<Value>(&large).unwrap()["id"],
        json!(1)
    );
    drop(large);

    send_json_value(
        &mut tui,
        json!({ "id": 2, "method": "config/read", "params": {} }),
    )
    .await;
    let follow_up = tui.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(follow_up.to_text().unwrap()).unwrap(),
        json!({ "id": 2, "result": {} })
    );
    finish(&mut tui, proxy).await;
    server.await.unwrap();
}

#[tokio::test]
async fn proxy_fails_only_the_request_whose_response_is_oversized() {
    let mut fixture = fixture("oversize-response");
    let proxy = spawn_proxy(&fixture);
    let listener = fixture.take_listener();
    let server = tokio::spawn(async move {
        let mut socket = accept_server(&listener).await;
        let first = next_json(&mut socket).await;
        let text = padded_text(
            &format!(r#"{{"id":{},"result":{{"catalog":""#, first["id"]),
            ABOVE_FORWARD_LIMIT_BYTES,
        );
        socket.send(Message::Text(text.into())).await.unwrap();
        let second = next_json(&mut socket).await;
        send_json_value(&mut socket, json!({ "id": second["id"], "result": {} })).await;
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
    });
    let mut tui = connect_tui(&fixture).await;
    send_json_value(
        &mut tui,
        json!({ "id": 1, "method": "plugin/list", "params": {} }),
    )
    .await;
    let failure = tui.next().await.unwrap().unwrap();
    let failure: Value = serde_json::from_str(failure.to_text().unwrap()).unwrap();
    assert_eq!(failure["id"], json!(1));
    assert_eq!(failure["error"]["code"], json!(-32603));
    assert_eq!(
        failure["error"]["data"]["reason"],
        "proxy_message_too_large"
    );
    assert!(failure.get("result").is_none());
    assert!(
        !proxy.is_finished(),
        "the proxy must survive the oversized response"
    );

    send_json_value(
        &mut tui,
        json!({ "id": 2, "method": "config/read", "params": {} }),
    )
    .await;
    let follow_up = tui.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(follow_up.to_text().unwrap()).unwrap(),
        json!({ "id": 2, "result": {} })
    );
    finish(&mut tui, proxy).await;
    server.await.unwrap();
}

#[tokio::test]
async fn proxy_rejects_an_oversized_server_request_and_drops_an_oversized_notification() {
    let mut fixture = fixture("oversize-server-initiated");
    let proxy = spawn_proxy(&fixture);
    let listener = fixture.take_listener();
    let server = tokio::spawn(async move {
        let mut socket = accept_server(&listener).await;
        let big_notification = padded_text(
            r#"{"method":"catalog/changed","params":{"data":""#,
            ABOVE_FORWARD_LIMIT_BYTES,
        );
        socket
            .send(Message::Text(big_notification.into()))
            .await
            .unwrap();
        let big_request = padded_text(
            r#"{"id":"srv-1","method":"catalog/ask","params":{"data":""#,
            ABOVE_FORWARD_LIMIT_BYTES,
        );
        socket
            .send(Message::Text(big_request.into()))
            .await
            .unwrap();
        send_json_value(
            &mut socket,
            json!({ "method": "catalog/ready", "params": {} }),
        )
        .await;
        let rejection = next_json(&mut socket).await;
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
        rejection
    });
    let mut tui = connect_tui(&fixture).await;
    let first = tui.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(first.to_text().unwrap()).unwrap(),
        json!({ "method": "catalog/ready", "params": {} })
    );
    assert!(
        !proxy.is_finished(),
        "the proxy must survive oversized server messages"
    );
    finish(&mut tui, proxy).await;

    let rejection = server.await.unwrap();
    assert_eq!(rejection["id"], "srv-1");
    assert_eq!(rejection["error"]["code"], json!(-32603));
    assert_eq!(
        rejection["error"]["data"]["reason"],
        "proxy_message_too_large"
    );
}

#[tokio::test]
async fn proxy_exit_keeps_the_final_error_line_for_session_diagnostics() {
    let mut fixture = fixture("proxy-exit-diagnostic");
    let session_dir = crate::session_dir(&fixture.context, &fixture.id);
    let diagnostic = session_dir.join(crate::STARTUP_DIAGNOSTIC_FILE);
    let context = fixture.context.clone();
    let args = fixture.args();
    let proxy = std::thread::spawn(move || run_proxy(&context, args));
    let listener = fixture.take_listener();
    let server = tokio::spawn(async move {
        let mut socket = accept_server(&listener).await;
        let _ = next_json(&mut socket).await;
        // Vanish without a closing handshake while the TUI stays connected.
        drop(socket);
    });
    let mut tui = connect_tui(&fixture).await;
    send_json_value(
        &mut tui,
        json!({ "id": 1, "method": "config/read", "params": {} }),
    )
    .await;
    server.await.unwrap();
    let status = tokio::task::spawn_blocking(move || proxy.join().unwrap())
        .await
        .unwrap();

    assert_eq!(status, nils_common::cli_contract::exit::RUNTIME);
    let recorded = fs::read_to_string(&diagnostic).unwrap();
    assert!(
        recorded.starts_with("error: Codex app-server proxy failed: upstream app-server"),
        "unexpected diagnostic: {recorded:?}"
    );
    assert_eq!(recorded.lines().count(), 1);
    assert_eq!(
        fs::metadata(&diagnostic).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
