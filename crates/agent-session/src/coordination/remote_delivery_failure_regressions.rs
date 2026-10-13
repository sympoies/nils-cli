use super::*;
use pretty_assertions::assert_eq;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Instant;

const SENDER: &str = "recipient";
const SENDER_INCARNATION: &str = "recipient-incarnation";
const TARGET: &str = "remote-recipient";
const TARGET_INCARNATION: &str = "remote-recipient-incarnation";
const ORIGINAL_BODY: &str = "original-fixture-body-canary";

enum RelayReply {
    Delivered,
    Unsupported { transient: bool },
}

/// Future repair-only requests need not occur before an intended RED assertion.
/// Drop stops and joins the bounded fixture without asserting during unwinding.
struct ScriptedRelay {
    config: Config,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    posts: Arc<Mutex<Vec<Value>>>,
    server: Option<JoinHandle<()>>,
}

impl ScriptedRelay {
    fn new(peers: Vec<Value>, replies: Vec<RelayReply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback fixture");
        listener.set_nonblocking(true).expect("bounded listener");
        let config = Config {
            machine: "source-machine".into(),
            url: format!("http://{}", listener.local_addr().unwrap()),
            token: "relay-fixture-token-00000000000000000001".into(),
            ingress_token: "ingress-fixture-token-0000000000000001".into(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let posts = Arc::new(Mutex::new(Vec::new()));
        let server_stop = stop.clone();
        let server_error = error.clone();
        let server_posts = posts.clone();
        let last_peer = peers.last().expect("at least one peer response").clone();
        let mut peers = VecDeque::from(peers);
        let mut replies = VecDeque::from(replies);
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut requests = 0;
            while !server_stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => {
                        *server_error.lock().unwrap() = Some("listener failed".into());
                        return;
                    }
                };
                requests += 1;
                if requests > 16 {
                    *server_error.lock().unwrap() = Some("request bound exceeded".into());
                    return;
                }
                let result = serve_request(
                    &mut stream,
                    &server_stop,
                    deadline,
                    &mut peers,
                    &last_peer,
                    &mut replies,
                    &server_posts,
                );
                if let Err(error) = result {
                    if !server_stop.load(Ordering::SeqCst) {
                        *server_error.lock().unwrap() = Some(error);
                    }
                    return;
                }
            }
        });
        Self {
            config,
            stop,
            error,
            posts,
            server: Some(server),
        }
    }

    fn assert_no_protocol_error(&self) {
        let error = self.error.lock().unwrap().clone();
        assert!(error.is_none(), "fixture protocol error: {error:?}");
    }

    fn posted_envelopes(&self) -> Vec<Value> {
        self.posts.lock().unwrap().clone()
    }
}

impl Drop for ScriptedRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn serve_request(
    stream: &mut TcpStream,
    stop: &AtomicBool,
    deadline: Instant,
    peers: &mut VecDeque<Value>,
    last_peer: &Value,
    replies: &mut VecDeque<RelayReply>,
    posts: &Mutex<Vec<Value>>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| "read timeout setup failed")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| "write timeout setup failed")?;
    let mut request = Vec::new();
    let (header_end, body_len) = loop {
        if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Err("fixture request deadline".into());
        }
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let header =
                std::str::from_utf8(&request[..end]).map_err(|_| "invalid request header")?;
            let length = header
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>())
                .transpose()
                .map_err(|_| "invalid content length")?
                .unwrap_or(0);
            if end + 4 + length > 8192 {
                return Err("request size bound exceeded".into());
            }
            if request.len() >= end + 4 + length {
                break (end, length);
            }
        }
        let mut bytes = [0; 1024];
        match stream.read(&mut bytes) {
            Ok(0) => return Err("incomplete fixture request".into()),
            Ok(count) => request.extend_from_slice(&bytes[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Err("fixture request read failed".into()),
        }
        if request.len() > 8192 {
            return Err("request size bound exceeded".into());
        }
    };
    let header =
        std::str::from_utf8(&request[..header_end]).map_err(|_| "invalid request header")?;
    let mut parts = header.lines().next().unwrap_or_default().split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    let (status, body) = match (method, path) {
        ("GET", "/api/coordination/peers/v1") => {
            (200, peers.pop_front().unwrap_or_else(|| last_peer.clone()))
        }
        ("POST", "/api/coordination/relay/v1") => {
            let envelope: Value =
                serde_json::from_slice(&request[header_end + 4..header_end + 4 + body_len])
                    .map_err(|_| "invalid fixture envelope")?;
            posts.lock().unwrap().push(envelope.clone());
            match replies.pop_front().ok_or("unexpected relay attempt")? {
                RelayReply::Delivered => (
                    200,
                    json!({
                        "schema_version": DELIVERY_VERSION,
                        "message_id": envelope["message_id"],
                        "state": "delivered",
                        "recipient": envelope["to"],
                        "persisted_at_epoch": now_epoch()
                    }),
                ),
                RelayReply::Unsupported { transient } => (
                    if transient { 503 } else { 403 },
                    json!({"error":{
                        "code":"remote-messaging-unsupported",
                        "details":{"messaging_unavailable_reason":
                            if transient { "coordination-unavailable" } else { "daemon-unsupported" }
                        }
                    }}),
                ),
            }
        }
        _ => return Err("unexpected fixture endpoint".into()),
    };
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|_| "fixture response write failed".into())
}

fn peer_snapshot(supported: bool, reason: Option<&str>) -> Value {
    json!({
        "schema_version":"agent-session.remote-peers.v1",
        "peers":[{
            "machine":"destination-machine", "session_id":TARGET,
            "session_incarnation":TARGET_INCARNATION,
            "messaging_supported":supported, "messaging_unavailable_reason":reason
        }]
    })
}

fn sender_fixture() -> (tempfile::TempDir, CliContext, PathBuf, String) {
    let (temp, context) = super::tests::fixture();
    let capability = super::super::capability_path(&context, SENDER, SENDER_INCARNATION);
    let token = std::fs::read_to_string(&capability).expect("synthetic fixture capability");
    (temp, context, capability, token)
}

fn submit_fixture(
    context: &CliContext,
    relay: &ScriptedRelay,
    token: &str,
) -> Result<Value, CliError> {
    submit(
        context,
        &relay.config,
        SENDER,
        token,
        Submit {
            to_machine: "destination-machine".into(),
            to_session: TARGET.into(),
            body: ORIGINAL_BODY.into(),
            idempotency_key: "remote-failure-fixture-key".into(),
            reply_to: None,
            expires_in: None,
            reply_revision: None,
            expected_recipient_incarnation: Some(TARGET_INCARNATION.into()),
            category: None,
            forward: None,
        },
    )
}

fn sender_inbox(context: &CliContext, capability: &Path) -> Value {
    super::super::mailbox::inbox(
        context,
        crate::cli::MessageInboxArgs {
            session: SENDER.into(),
            capability_file: Some(capability.to_path_buf()),
            state: None,
            cursor: None,
            limit: Some(50),
            format: nils_common::cli_contract::OutputFormat::Json,
            categories: Vec::new(),
        },
    )
    .expect("authenticated fixture sender inbox")
}

fn replace_synthetic_sender_incarnation(context: &CliContext) -> PathBuf {
    let replacement = "replacement-sender-incarnation";
    let token = "replacement-fixture-capability-00000000000001";
    let record_path = context.state_dir.join("sessions/recipient/session.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!(replacement);
    record["runtime"]["generation"] = json!(2);
    write_atomic(
        &record_path,
        &serde_json::to_vec(&record).unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    let capability = super::super::capability_path(context, SENDER, replacement);
    write_atomic(&capability, token.as_bytes(), SECRET_FILE_MODE).unwrap();
    write_atomic(
        &context
            .state_dir
            .join("sessions/recipient/coordination/heartbeat"),
        format!("{replacement}:{}\n", now_epoch()).as_bytes(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    let mut registry = lock_registry(context).unwrap();
    let sender = registry.registry.brokers.get_mut(SENDER).unwrap();
    sender.incarnation = replacement.into();
    sender.generation = 2;
    sender.capability_digest = digest_bytes(token.as_bytes());
    sender.heartbeat_epoch = now_epoch();
    sender.heartbeat_at = timestamp(now_epoch());
    registry.save().unwrap();
    capability
}

#[test]
fn capability_lookup_recovers_transient_unavailable_peer() {
    let (_temp, context, _capability, token) = sender_fixture();
    let relay = ScriptedRelay::new(
        vec![
            peer_snapshot(false, Some("coordination-unavailable")),
            peer_snapshot(true, None),
        ],
        Vec::new(),
    );
    let queued = submit_fixture(&context, &relay, &token)
        .expect("transient capability lookup must re-resolve the capable peer");
    assert_eq!(queued["state"], "queued");
    assert_eq!(
        queued["recipient"]["session_incarnation"],
        TARGET_INCARNATION
    );
    relay.assert_no_protocol_error();
}

#[test]
fn queued_transient_capability_rejection_retries_same_envelope() {
    let (_temp, context, _capability, token) = sender_fixture();
    let relay = ScriptedRelay::new(
        vec![
            peer_snapshot(true, None),
            peer_snapshot(false, Some("coordination-unavailable")),
            peer_snapshot(true, None),
        ],
        vec![
            RelayReply::Unsupported { transient: true },
            RelayReply::Delivered,
        ],
    );
    let queued = submit_fixture(&context, &relay, &token).unwrap();
    let id = queued["message_id"].as_str().unwrap();
    let recipient = queued["recipient"].clone();
    drain(&context, &relay.config).unwrap();
    let pending = delivery(&context, SENDER, &token, id).unwrap();
    assert_eq!(
        pending["state"], "queued",
        "transient capability failure became terminal"
    );
    assert_eq!(pending["message_id"], queued["message_id"]);
    assert_eq!(pending["recipient"], recipient);
    {
        let mut journal = lock_journal(&context).unwrap();
        let owned = journal
            .registry
            .remote_outbox
            .iter_mut()
            .find(|entry| entry.envelope.message_id == id)
            .unwrap();
        owned.next_attempt_epoch = now_epoch();
        journal.save().unwrap();
    }
    drain(&context, &relay.config).unwrap();
    let delivered = delivery(&context, SENDER, &token, id).unwrap();
    assert_eq!(delivered["state"], "delivered");
    let posts = relay.posted_envelopes();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0], posts[1],
        "retry retargeted or rebuilt the original envelope"
    );
    relay.assert_no_protocol_error();
}

#[test]
fn terminal_remote_rejection_is_visible_to_sender() {
    let (_temp, context, capability, token) = sender_fixture();
    let relay = ScriptedRelay::new(
        vec![
            peer_snapshot(true, None),
            peer_snapshot(false, Some("daemon-unsupported")),
        ],
        vec![RelayReply::Unsupported { transient: false }],
    );
    let queued = submit_fixture(&context, &relay, &token).unwrap();
    let id = queued["message_id"].as_str().unwrap();
    drain(&context, &relay.config).unwrap();
    let rejected = delivery(&context, SENDER, &token, id).unwrap();
    assert_eq!(rejected["state"], "rejected");
    assert_eq!(rejected["reason"], "remote-messaging-unsupported");
    let inbox = sender_inbox(&context, &capability);
    let notices = inbox["messages"].as_array().unwrap();
    assert_eq!(
        notices.len(),
        1,
        "terminal rejection was not visible in the sender mailbox"
    );
    let notice_id = notices[0]["message_id"].as_str().unwrap();
    let shown = super::super::mailbox::show(
        &context,
        crate::cli::MessageShowArgs {
            session: SENDER.into(),
            message: notice_id.into(),
            capability_file: Some(capability.clone()),
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    )
    .unwrap();
    let text = shown["body"]["text"].as_str().unwrap();
    assert!(text.contains(id));
    assert!(text.contains("remote-messaging-unsupported"));
    assert!(text.contains("message delivery"));
    assert!(text.to_ascii_lowercase().contains("retry"));
    assert!(text.len() <= 4096);
    assert!(!text.contains(ORIGINAL_BODY));
    assert!(!text.contains(&token));
    drain(&context, &relay.config).unwrap();
    assert_eq!(
        sender_inbox(&context, &capability)["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let replacement_capability = replace_synthetic_sender_incarnation(&context);
    drain(&context, &relay.config).unwrap();
    assert!(
        sender_inbox(&context, &replacement_capability)["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    relay.assert_no_protocol_error();
}

#[test]
fn supported_peer_delivery_has_no_failure_notice() {
    let (_temp, context, capability, token) = sender_fixture();
    let relay = ScriptedRelay::new(vec![peer_snapshot(true, None)], vec![RelayReply::Delivered]);
    let queued = submit_fixture(&context, &relay, &token).unwrap();
    let id = queued["message_id"].as_str().unwrap();
    drain(&context, &relay.config).unwrap();
    assert_eq!(
        delivery(&context, SENDER, &token, id).unwrap()["state"],
        "delivered"
    );
    assert!(
        sender_inbox(&context, &capability)["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    relay.assert_no_protocol_error();
}

#[test]
fn definitively_unsupported_peer_is_refused_before_enqueue() {
    let (_temp, context, _capability, token) = sender_fixture();
    let relay = ScriptedRelay::new(
        vec![peer_snapshot(false, Some("daemon-unsupported"))],
        Vec::new(),
    );
    let refused = submit_fixture(&context, &relay, &token).unwrap_err();
    assert_eq!(refused.code(), "remote-messaging-unsupported");
    assert!(
        lock_journal(&context)
            .unwrap()
            .registry
            .remote_outbox
            .is_empty()
    );
    relay.assert_no_protocol_error();
}
