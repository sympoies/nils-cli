//! Real origin and destination daemons with a loopback, spec-conforming relay stub.
use super::*;
use agent_session::cli::resume_move::sha256;
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

const CALLER_TOKEN: &str = "synthetic-caller-capability-000001";
const RELAY_TOKEN: &str = "synthetic-relay-outbound-000000001";
const RELAY_ROUTE: &str = "/api/coordination/resume-move/v1";
const NEXT_OPERATION: &str = "01900000-0000-7000-8000-000000000021";
const UNKNOWN: &str = "resume-move-outcome-unknown";

struct Origin<'a> {
    root: PathBuf,
    state: PathBuf,
    capability: PathBuf,
    fixture: &'a Fixture<'a>,
}

impl<'a> Origin<'a> {
    fn new(fixture: &'a Fixture<'a>) -> Self {
        let root = fixture.root.join("origin");
        let state = root.join("state");
        fs::create_dir(&root).unwrap();
        // The common serve fixture requires a tmux stub inside the fixture root.
        fs::copy(fixture.tmux, root.join("tmux")).unwrap();
        let mut origin = Self {
            root,
            state,
            capability: PathBuf::new(),
            fixture,
        };
        origin.seed_caller(CALLER_INCARNATION);
        origin
    }

    fn seed_caller(&mut self, incarnation: &str) {
        let session = self.state.join("sessions/caller");
        let coordination = session.join("coordination");
        fs::create_dir_all(&coordination).unwrap();
        for dir in [&session, &coordination, &self.state.join("coordination")] {
            fs::create_dir_all(dir).unwrap();
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let capability =
            coordination.join(format!("capability-{}", sha256(incarnation.as_bytes())));
        let token = if incarnation == CALLER_INCARNATION {
            CALLER_TOKEN
        } else {
            "synthetic-replacement-capability-01"
        };
        write_private(&capability, token.as_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        write_private(
            &coordination.join("heartbeat"),
            format!("{incarnation}:{now}\n").as_bytes(),
        );
        write_private(
            &self.state.join("coordination/registry.json"),
            json!({
                "schema_version":"agent-session.coordination-registry.v1",
                "brokers":{"caller":{"session_id":"caller","incarnation":incarnation,
                    "capability_digest":sha256(token.as_bytes()),"generation":1,"state":"ready",
                    "heartbeat_at":"2026-01-01T00:00:00Z","heartbeat_epoch":now}}
            })
            .to_string()
            .as_bytes(),
        );
        write_private(&session.join("session.json"), json!({
            "schema_version":"agent-session.session.v1","id":"caller","agent":"claude",
            "mode":"interactive","cwd":self.root,"tmux_session":"fixture-caller","created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-01T00:00:00Z",
            "runtime":{"kind":"tmux","tmux_session":"fixture-caller","generation":1,
                "started_at":"2026-01-01T00:00:00Z","launch_id":incarnation},
            "tmux_runtime_never_launched":incarnation
        }).to_string().as_bytes());
        self.capability = capability;
    }

    fn daemon(&self, relay: Option<&str>) -> Daemon {
        let address = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut command = crate::serve_fixture::command(
            &self.root,
            nils_test_support::bin::resolve("agent-session"),
            &self.state,
            &self.root.join("tmux"),
            |command| {
                command
                    .args([
                        "--bind",
                        &address.to_string(),
                        "--machine",
                        "origin",
                        "--token",
                        BEARER,
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null());
                if let Some(relay) = relay {
                    command
                        .env("AGENT_SESSION_RELAY_URL", relay)
                        .env("AGENT_SESSION_RELAY_TOKEN", RELAY_TOKEN)
                        .env("AGENT_SESSION_RELAY_INGRESS_TOKEN", INGRESS);
                }
            },
        );
        let mut daemon = Daemon {
            child: command.spawn().unwrap(),
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap(),
            base: format!("http://{address}"),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(address).is_err() {
            assert!(daemon.child.try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline, "origin daemon did not listen");
            thread::sleep(Duration::from_millis(10));
        }
        let endpoint = self.state.join("coordination/daemon-endpoint.json");
        while !fs::read(&endpoint)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|published| published["url"] == daemon.base)
        {
            assert!(daemon.child.try_wait().unwrap().is_none());
            assert!(
                Instant::now() < deadline,
                "origin endpoint was not published"
            );
            thread::sleep(Duration::from_millis(10));
        }
        daemon
    }

    fn start(&self, operation: &str) -> Value {
        self.start_with_account(operation, "fixture-account")
    }

    fn start_with_account(&self, operation: &str, account: &str) -> Value {
        run_resolved(
            "agent-session",
            &[
                "--state-dir",
                self.state.to_str().unwrap(),
                "start",
                "--agent",
                "claude",
                "--via-console",
                "--machine",
                "destination",
                "--resume",
                &self.fixture.proof.binding.provider_session_id,
                "--move-id",
                self.fixture.move_id(),
                "--account",
                account,
                "--cwd",
                self.fixture.cwd.to_str().unwrap(),
                "--operation-id",
                operation,
                "--format",
                "json",
            ],
            &CmdOptions::new()
                .with_env_remove_prefix("AGENT_SESSION_")
                .with_env("HOME", self.root.to_str().unwrap())
                .with_env("AGENT_SESSION_ID", "caller")
                .with_env(
                    "AGENT_SESSION_CAPABILITY_FILE",
                    self.capability.to_str().unwrap(),
                ),
        )
        .stdout_json()
    }
}

#[derive(Clone, Copy)]
enum Reply {
    Forward,
    DropFirst,
    InvalidFirst,
    NoCapability,
}

#[derive(Default)]
struct Traffic {
    handshakes: usize,
    posts: Vec<Vec<u8>>,
    destination_starts: Vec<Vec<u8>>,
    destination_reads: usize,
    terminal: Option<(u16, Value)>,
}

struct Relay {
    url: String,
    traffic: Arc<Mutex<Traffic>>,
    shutdown: std::sync::mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Relay {
    fn spawn(f: &Fixture<'_>, target: &Daemon, reply: Reply) -> Self {
        let start_route = format!("{}{START}", target.base);
        let listing = format!("{}/sessions", target.base);
        let status_route = format!("{}{}", target.base, f.status_path());
        let start_binding = f.binding("start");
        let read_binding = f.binding("read");
        let client = target.client.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let (shutdown, stop) = std::sync::mpsc::channel();
        let traffic = Arc::new(Mutex::new(Traffic::default()));
        let recorded = Arc::clone(&traffic);
        let worker = thread::spawn(move || {
            loop {
                match stop.try_recv() {
                    Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                let (line, body) = request(&mut stream);
                let mut traffic = recorded.lock().unwrap();
                let (status, result) = if line == format!("GET {RELAY_ROUTE} HTTP/1.1") {
                    traffic.handshakes += 1;
                    if matches!(reply, Reply::NoCapability) {
                        (404, Value::Null)
                    } else {
                        (
                            200,
                            json!({"schema_version":"agent-session.console-resume-move-capability.v1",
                        "ok":true,"data":{"machine":"origin","capability":"resume-move-start/v1"}}),
                        )
                    }
                } else {
                    assert_eq!(line, format!("POST {RELAY_ROUTE} HTTP/1.1"));
                    traffic.posts.push(body.clone());
                    if let Some(result) = traffic.terminal.clone() {
                        result
                    } else if traffic.posts.len() > 1 {
                        assert_eq!(
                            body, traffic.posts[0],
                            "reconcile must resend the retained bytes"
                        );
                        traffic.destination_reads += 1;
                        let read = bind(client.get(&status_route), &read_binding)
                            .send()
                            .unwrap();
                        let data = response(read, 200)["data"].clone();
                        if data["state"] == "created" {
                            (
                                200,
                                json!({"schema_version":SCHEMA,"ok":true,"data":{
                                    "machine":data["machine"],"move_id":data["move_id"],"state":"created",
                                    "session_id":data["session_id"],"session_incarnation":data["session_incarnation"]
                                }}),
                            )
                        } else {
                            (
                                502,
                                refusal(
                                    SCHEMA,
                                    UNKNOWN,
                                    json!({"move_id":data["move_id"],
                            "safe_to_retry":false,"next_action":"reconcile_destination"}),
                                ),
                            )
                        }
                    } else if matches!(reply, Reply::InvalidFirst) {
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        (
                            502,
                            refusal(
                                SCHEMA,
                                "resume-move-invented",
                                json!({
                        "move_id":request["session"]["move_id"],"safe_to_retry":false,
                        "next_action":"reconcile_destination"}),
                            ),
                        )
                    } else {
                        let data = response(
                            client.get(&listing).bearer_auth(BEARER).send().unwrap(),
                            200,
                        );
                        if data["data"]["capabilities"]["resume_move_start_v1"] != true {
                            let request: Value = serde_json::from_slice(&body).unwrap();
                            (
                                503,
                                refusal(
                                    SCHEMA,
                                    "resume-move-unavailable",
                                    json!({
                            "move_id":request["session"]["move_id"],"session_created":false,
                            "safe_to_retry":false,"next_action":"inspect_destination"}),
                                ),
                            )
                        } else {
                            let mut binding = start_binding.clone();
                            let request: Value = serde_json::from_slice(&body).unwrap();
                            binding["operation_id"] = request["operation_id"].clone();
                            traffic.destination_starts.push(body.clone());
                            let response = bind(
                                client
                                    .post(&start_route)
                                    .header("content-type", "application/json")
                                    .body(body),
                                &binding,
                            )
                            .send()
                            .unwrap();
                            let status = response.status().as_u16();
                            let result: Value = response.json().unwrap();
                            if matches!(reply, Reply::DropFirst) {
                                assert_eq!(status, 200);
                                continue;
                            }
                            traffic.terminal = Some((status, result.clone()));
                            (status, result)
                        }
                    }
                };
                let payload = result.to_string();
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
            }
        });
        Self {
            url,
            traffic,
            shutdown,
            worker: Some(worker),
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

fn request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut length = 0;
    let mut authorized = false;
    loop {
        let mut header = String::new();
        assert!(reader.read_line(&mut header).unwrap() > 0);
        if header == "\r\n" {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        match name.to_ascii_lowercase().as_str() {
            "authorization" => {
                assert_eq!(value.trim(), format!("Bearer {RELAY_TOKEN}"));
                authorized = true;
            }
            "content-length" => length = value.trim().parse().unwrap(),
            BINDING_HEADER => panic!("origin must not send the relay binding"),
            _ => {}
        }
    }
    assert!(authorized);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (line.trim_end().to_owned(), body)
}

#[test]
fn console_origin_forwards_the_full_request_to_destination() {
    with_console_fixture(|f| {
        let target = f.daemon(true, Some(&f.signers.allowlist), false);
        let relay = Relay::spawn(&f, &target, Reply::Forward);
        let origin = Origin::new(&f);
        let _daemon = origin.daemon(Some(&relay.url));
        let result = origin.start(OPERATION);
        assert_eq!(result["data"]["state"], "created", "{result}");
        assert_eq!(
            result["data"]["session_id"],
            format!("move-{}", f.move_id())
        );
        let traffic = relay.traffic.lock().unwrap();
        assert_eq!(traffic.destination_starts, traffic.posts);
        assert_eq!(traffic.destination_starts.len(), 1);
        assert_eq!(
            traffic.destination_starts[0],
            serde_json::to_vec(&f.request()).unwrap()
        );
        assert_eq!(f.launch_count(), 1);
    });
}

#[test]
fn console_origin_missing_capability_has_no_fallback_post() {
    with_console_fixture(|f| {
        let target = f.daemon(false, Some(&f.signers.allowlist), false);
        let origin = Origin::new(&f);
        for (configured, reply, gets, posts) in [
            (false, Reply::Forward, 0, 0),
            (true, Reply::NoCapability, 1, 0),
            (true, Reply::Forward, 1, 1),
        ] {
            let relay = Relay::spawn(&f, &target, reply);
            let _daemon = origin.daemon(configured.then_some(relay.url.as_str()));
            let result = origin.start(OPERATION);
            assert_eq!(result["error"]["code"], "resume-move-unavailable");
            assert_eq!(result["error"]["details"]["session_created"], false);
            let traffic = relay.traffic.lock().unwrap();
            assert_eq!(traffic.handshakes, gets);
            assert_eq!(traffic.posts.len(), posts);
            assert!(traffic.destination_starts.is_empty());
        }
        assert_eq!(f.launch_count(), 0);
    });
}

#[test]
fn console_origin_preserves_a_typed_destination_refusal() {
    with_console_fixture(|f| {
        let target = f.daemon(true, Some(&f.signers.allowlist), false);
        let relay = Relay::spawn(&f, &target, Reply::Forward);
        let origin = Origin::new(&f);
        let daemon = origin.daemon(Some(&relay.url));
        let result = origin.start_with_account(OPERATION, "other-account");
        assert_eq!(
            result["error"]["code"], "resume-account-mismatch",
            "{result}"
        );
        let traffic = relay.traffic.lock().unwrap();
        let (status, upstream) = traffic.terminal.as_ref().unwrap();
        assert_eq!(*status, 422);
        assert_eq!(result["error"]["details"], upstream["error"]["details"]);
        let (body, expected) = (traffic.posts[0].clone(), upstream.clone());
        drop(traffic);

        let retained = daemon
            .client
            .post(format!(
                "{}/sessions/caller/console-resume-move/v1",
                daemon.base
            ))
            .bearer_auth(CALLER_TOKEN)
            .body(body)
            .send()
            .unwrap();
        assert_eq!(response(retained, 422), expected);
        assert_eq!(relay.traffic.lock().unwrap().posts.len(), 1);
        assert_eq!(f.launch_count(), 0);
    });
}

#[test]
fn console_origin_dropped_reply_reconciles_read_only_after_origin_restart() {
    with_console_fixture(|f| {
        let target = f.daemon(true, Some(&f.signers.allowlist), false);
        let relay = Relay::spawn(&f, &target, Reply::DropFirst);
        let origin = Origin::new(&f);
        {
            let _daemon = origin.daemon(Some(&relay.url));
            assert_unknown(&origin.start(OPERATION));
        }
        let ledger = f.ledger();
        let _daemon = origin.daemon(Some(&relay.url));
        let result = origin.start(NEXT_OPERATION);
        let id = format!("move-{}", f.move_id());
        let record: Value = serde_json::from_slice(
            &fs::read(f.state.join("sessions").join(&id).join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(result["data"]["session_id"], id);
        assert_eq!(
            result["data"]["session_incarnation"],
            record["runtime"]["launch_id"]
        );
        assert_eq!(origin.start(OPERATION), result);
        let traffic = relay.traffic.lock().unwrap();
        assert_eq!(traffic.posts.len(), 2);
        assert_eq!(traffic.posts[0], traffic.posts[1]);
        assert_eq!(traffic.destination_reads, 1);
        assert_eq!(traffic.destination_starts.len(), 1);
        assert_eq!(f.launch_count(), 1);
        assert_eq!(fs::read_dir(f.state.join("sessions")).unwrap().count(), 1);
        assert_eq!(f.ledger(), ledger);
    });
}

#[test]
fn console_origin_unknown_outcome_never_reforwards() {
    with_console_fixture(|f| {
        let target = f.daemon(true, Some(&f.signers.allowlist), false);
        let relay = Relay::spawn(&f, &target, Reply::InvalidFirst);
        let origin = Origin::new(&f);
        let baseline = f.ledger();
        {
            let _daemon = origin.daemon(Some(&relay.url));
            assert_unknown(&origin.start(OPERATION));
        }
        let unavailable = Relay::spawn(&f, &target, Reply::NoCapability);
        {
            let _daemon = origin.daemon(Some(&unavailable.url));
            assert_unknown(&origin.start(NEXT_OPERATION));
            assert!(unavailable.traffic.lock().unwrap().posts.is_empty());
        }
        let _daemon = origin.daemon(Some(&relay.url));
        assert_unknown(&origin.start(OPERATION));
        assert_unknown(&origin.start(NEXT_OPERATION));
        let traffic = relay.traffic.lock().unwrap();
        assert_eq!(traffic.posts.len(), 3);
        assert_eq!(traffic.destination_reads, 2);
        assert!(traffic.destination_starts.is_empty());
        assert_eq!(f.launch_count(), 0);
        assert_eq!(f.ledger(), baseline);
        drop(traffic);

        write_private(
            &origin.state.join("coordination/resume-move-origin.v1.json"),
            b"invalid",
        );
        let corrupt = origin.start(OPERATION);
        assert_eq!(corrupt["error"]["code"], "resume-move-state-unavailable");
        assert!(corrupt["error"]["details"].get("session_created").is_none());
        assert_eq!(relay.traffic.lock().unwrap().posts.len(), 3);

        fs::remove_file(origin.state.join("coordination/resume-move-origin.v1.json")).unwrap();
        let missing = origin.start(NEXT_OPERATION);
        assert_eq!(missing["error"]["code"], "resume-move-state-unavailable");
        assert!(missing["error"]["details"].get("session_created").is_none());
        assert_eq!(relay.traffic.lock().unwrap().posts.len(), 3);
    });
}

#[test]
fn console_origin_replacement_caller_cannot_reconcile() {
    with_console_fixture(|f| {
        let target = f.daemon(true, Some(&f.signers.allowlist), false);
        let relay = Relay::spawn(&f, &target, Reply::InvalidFirst);
        let mut origin = Origin::new(&f);
        {
            let _daemon = origin.daemon(Some(&relay.url));
            assert_unknown(&origin.start(OPERATION));
        }
        origin.seed_caller("00000000-0000-4000-8000-000000000031");
        let _daemon = origin.daemon(Some(&relay.url));
        let conflict = origin.start(NEXT_OPERATION);
        assert_eq!(conflict["error"]["code"], "resume-move-binding-mismatch");
        let traffic = relay.traffic.lock().unwrap();
        assert_eq!(traffic.handshakes, 1);
        assert_eq!(traffic.posts.len(), 1);
        assert_eq!(traffic.destination_reads, 0);
        assert!(traffic.destination_starts.is_empty());
    });
}

fn assert_unknown(result: &Value) {
    assert_eq!(result["error"]["code"], UNKNOWN, "{result}");
    let details = &result["error"]["details"];
    assert_eq!(details["safe_to_retry"], false);
    assert_eq!(details["next_action"], "reconcile_destination");
    assert!(details.get("session_created").is_none());
}
