//! Session board v1 relay mode (`docs/specs/session-board-v1.md`, "Relay
//! route" and "Mode selection"): a managed session's `agent-session board`
//! asks the aggregator through the daemon's `GET /sessions/{id}/board/v1`,
//! here against a fake aggregator. The same harness covers owned child
//! sessions (`docs/specs/session-coordination-v1.md`, "Owned child
//! sessions"): `agent-session start --via-console` through the daemon's
//! `POST /sessions/{id}/console-start/v1`.

use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const OPERATOR: &str = "board-relay-operator-token-0000000000000001";
const RELAY_TOKEN: &str = "board-relay-service-token-00000000000000001";
const INGRESS_TOKEN: &str = "board-relay-ingress-token-00000000000000001";
const CAPABILITY: &str = "board-relay-session-capability-000000000000001";
const MACHINE: &str = "host-a";
const SESSION: &str = "20300101-000000-self";
const INCARNATION: &str = "self-incarnation";

const ISOLATED_ENV: [&str; 6] = [
    "AGENT_SESSION_BOARD",
    "AGENT_SESSION_MACHINE",
    "AGENT_SESSION_HOST",
    "AGENT_SESSION_RELAY_URL",
    "AGENT_SESSION_RELAY_TOKEN",
    "AGENT_SESSION_RELAY_INGRESS_TOKEN",
];

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).expect("private dir");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("dir mode");
}

/// The `lineage` the daemon adds for a child of the fixture session
/// (`docs/specs/session-lineage-work-v1.md`, "Console starts").
fn child_lineage() -> Value {
    let caller = json!({
        "machine": MACHINE,
        "session_id": SESSION,
        "session_created_at": "2030-01-01T00:00:00Z",
    });
    let mut parent = caller.clone();
    parent["session_incarnation"] = json!(INCARNATION);
    json!({
        "schema_version": "agent-session.session-lineage.v1",
        "parent": parent,
        "root": caller,
        "depth": 1,
        "starter": {"kind": "session", "via": "console"},
    })
}

fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("private file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("file mode");
}

/// One recorded aggregator request.
#[derive(Clone, Debug)]
struct Seen {
    target: String,
    authorization: Option<String>,
    body: Option<Value>,
}

impl Seen {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    fn query(&self) -> Vec<(String, String)> {
        let url = reqwest::Url::parse(&format!("http://fixture{}", self.target)).expect("target");
        url.query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }
}

#[derive(Default)]
struct AggregatorState {
    replies: VecDeque<String>,
    seen: Vec<Seen>,
}

/// A loopback HTTP/1.1 aggregator that answers each request with the next
/// queued raw response and records what it was asked.
struct Aggregator {
    url: String,
    state: Arc<Mutex<AggregatorState>>,
}

impl Aggregator {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind aggregator");
        let url = format!("http://{}", listener.local_addr().expect("aggregator addr"));
        let state = Arc::new(Mutex::new(AggregatorState::default()));
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") && head.len() < 64 * 1024 {
                    match stream.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                let head = String::from_utf8_lossy(&head).to_string();
                let target = head
                    .lines()
                    .next()
                    .and_then(|line| line.split(' ').nth(1))
                    .unwrap_or_default()
                    .to_string();
                let header = |wanted: &str| {
                    head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case(wanted)
                            .then(|| value.trim().to_string())
                    })
                };
                let authorization = header("authorization");
                let length = header("content-length")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0_u8; length];
                let body = stream
                    .read_exact(&mut body)
                    .ok()
                    .and_then(|()| serde_json::from_slice::<Value>(&body).ok());
                let reply = {
                    let mut state = shared.lock().expect("aggregator state");
                    state.seen.push(Seen {
                        target,
                        authorization,
                        body,
                    });
                    state.replies.pop_front()
                };
                let reply = reply.unwrap_or_else(|| raw(500, "{}", ""));
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Self { url, state }
    }

    fn reply(&self, response: String) {
        self.state
            .lock()
            .expect("aggregator state")
            .replies
            .push_back(response);
    }

    fn reply_json(&self, status: u16, body: &Value) {
        self.reply(raw(status, &body.to_string(), ""));
    }

    fn seen(&self) -> Vec<Seen> {
        self.state.lock().expect("aggregator state").seen.clone()
    }
}

fn raw(status: u16, body: &str, extra_headers: &str) -> String {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
        body.len()
    )
}

/// A state directory with one managed session whose broker is ready.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    state_dir: PathBuf,
    home: PathBuf,
    capability_file: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let state_dir = root.join("state");
        let home = root.join("home");
        let cwd = home.join("Project/nils-cli");
        fs::create_dir_all(&cwd).expect("session cwd");
        private_dir(&state_dir);
        private_dir(&state_dir.join("sessions"));
        let session_dir = state_dir.join("sessions").join(SESSION);
        private_dir(&session_dir.join("coordination"));
        let record = json!({
            "schema_version": "agent-session.session.v1",
            "id": SESSION,
            "agent": "claude",
            "mode": "interactive",
            "title": "Relay fixture",
            "title_revision": 0,
            "cwd": cwd.to_string_lossy(),
            "tmux_session": "agent-self",
            "prompt_file": null,
            "log_file": null,
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z",
            "coordination_mode": "advisory",
            "runtime": {
                "kind": "tmux",
                "tmux_session": "agent-self",
                "generation": 1,
                "started_at": "2030-01-01T00:00:00Z",
                "launch_id": INCARNATION
            }
        });
        private_file(
            &session_dir.join("session.json"),
            &serde_json::to_vec(&record).expect("record"),
        );
        let capability_file = session_dir
            .join("coordination")
            .join(format!("capability-{}", digest(INCARNATION)));
        private_file(&capability_file, CAPABILITY.as_bytes());
        private_dir(&state_dir.join("coordination"));
        let registry = json!({
            "schema_version": "agent-session.coordination-registry.v2",
            "fingerprint_epoch": 1,
            "fingerprint_key": "fixture-private-fingerprint-key-material-0000000001",
            "brokers": {SESSION: {
                "session_id": SESSION,
                "incarnation": INCARNATION,
                "coordination_mode": "advisory",
                "capability_digest": digest(CAPABILITY),
                "generation": 1,
                "state": "ready",
                "heartbeat_at": "2030-01-01T00:00:00Z",
                "heartbeat_epoch": now_epoch()
            }},
            "claims": [],
            "operations": [],
            "messages": [],
            "receipts": {},
            "notifications": {}
        });
        private_file(
            &state_dir.join("coordination/registry.json"),
            &serde_json::to_vec(&registry).expect("registry"),
        );
        let fixture = Self {
            _tmp: tmp,
            root,
            state_dir,
            home,
            capability_file,
        };
        fixture.heartbeat();
        fixture
    }

    /// The broker heartbeat is fresh for 30 seconds; refresh it per call.
    fn heartbeat(&self) {
        private_file(
            &self
                .state_dir
                .join("sessions")
                .join(SESSION)
                .join("coordination/heartbeat"),
            format!("{INCARNATION}:{}\n", now_epoch()).as_bytes(),
        );
    }

    /// Written once: rewriting it while a daemon runs it fails with ETXTBSY.
    fn fake_tmux(&self) -> PathBuf {
        let bin = self.root.join("tmux");
        if !bin.exists() {
            fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("fake tmux");
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("fake tmux mode");
        }
        bin
    }

    /// `agent-session board` as the managed session.
    fn board(&self, args: &[&str]) -> CmdOutput {
        self.run("board", args, true)
    }

    /// `agent-session start --via-console`, as the managed session or, with
    /// `managed` false, from an unmanaged shell.
    fn start_via_console(&self, args: &[&str], managed: bool) -> CmdOutput {
        let mut argv = vec!["--via-console"];
        argv.extend_from_slice(args);
        self.run("start", &argv, managed)
    }

    fn run(&self, command: &str, args: &[&str], managed: bool) -> CmdOutput {
        self.heartbeat();
        let state = self.state_dir.to_string_lossy().to_string();
        let tmux = self.fake_tmux().to_string_lossy().to_string();
        let home = self.home.to_string_lossy().to_string();
        let capability = self.capability_file.to_string_lossy().to_string();
        let mut argv = vec!["--state-dir", state.as_str(), command];
        argv.extend_from_slice(args);
        let mut envs = vec![
            ("HOME", home.as_str()),
            ("AGENT_SESSION_TMUX_BIN", tmux.as_str()),
            ("AGENT_SESSION_MACHINE", MACHINE),
        ];
        if managed {
            envs.push(("AGENT_SESSION_ID", SESSION));
            envs.push(("AGENT_SESSION_CAPABILITY_FILE", capability.as_str()));
        }
        let options = CmdOptions::new()
            .with_cwd(&self.root)
            .without_ambient_managed_session_env()
            .with_env_remove_many(&ISOLATED_ENV)
            .with_envs(&envs);
        run_resolved("agent-session", &argv, &options)
    }

    fn serve(&self, args: &[&str], relay: Option<&Aggregator>) -> Serve {
        let endpoint = self.state_dir.join("coordination/daemon-endpoint.json");
        let _ = fs::remove_file(&endpoint);
        let stderr_path = self.root.join("serve.stderr");
        let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
        command
            .arg("serve")
            .arg("--bind")
            .arg("127.0.0.1:0")
            .arg("--state-dir")
            .arg(&self.state_dir)
            .arg("--machine")
            .arg(MACHINE)
            .args(args)
            .env("HOME", &self.home)
            .env("AGENT_SESSION_TOKEN", OPERATOR)
            .env("AGENT_SESSION_TMUX_BIN", self.fake_tmux())
            .env_remove("XDG_STATE_HOME")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(&stderr_path).expect("serve stderr"),
            ));
        for key in nils_test_support::cmd::MANAGED_SESSION_ENV
            .iter()
            .chain(ISOLATED_ENV.iter())
        {
            command.env_remove(key);
        }
        if let Some(aggregator) = relay {
            command
                .env("AGENT_SESSION_RELAY_URL", &aggregator.url)
                .env("AGENT_SESSION_RELAY_TOKEN", RELAY_TOKEN)
                .env("AGENT_SESSION_RELAY_INGRESS_TOKEN", INGRESS_TOKEN);
        }
        let mut child = command.spawn().expect("spawn serve");
        let deadline = Instant::now() + Duration::from_secs(15);
        let addr = loop {
            let published = fs::read(&endpoint).ok().and_then(|raw| {
                let url = serde_json::from_slice::<Value>(&raw).ok()?["url"]
                    .as_str()?
                    .to_string();
                url.strip_prefix("http://")?.parse::<SocketAddr>().ok()
            });
            if let Some(addr) = published {
                break addr;
            }
            if let Some(status) = child.try_wait().expect("poll serve") {
                panic!(
                    "serve exited before listening: {status}; {}",
                    fs::read_to_string(&stderr_path).unwrap_or_default()
                );
            }
            assert!(
                Instant::now() < deadline,
                "serve did not publish its endpoint"
            );
            thread::sleep(Duration::from_millis(20));
        };
        Serve { child, addr }
    }
}

/// Kills its serve on drop, so a failed assertion cannot orphan a daemon.
struct Serve {
    child: Child,
    addr: SocketAddr,
}

impl Serve {
    fn get(&self, path: &str, bearer: Option<&str>) -> (u16, Value) {
        let mut request =
            reqwest::blocking::Client::new().get(format!("http://{}{path}", self.addr));
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        let response = request.send().expect("serve request");
        let status = response.status().as_u16();
        (status, response.json::<Value>().expect("json body"))
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        if self.child.try_wait().expect("poll serve").is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn record(machine: &str, id: &str, incarnation: Option<&str>, messaging: bool) -> Value {
    json!({
        "machine": machine,
        "session_id": id,
        "session_incarnation": incarnation,
        "messaging_supported": messaging,
        "repo_name": "nils-cli",
        "cwd": "~/Project/nils-cli",
        "provider": "claude",
        "agent_profile": null,
        "title": format!("Title {id}"),
        "title_state": null,
        "turn_state": {
            "phase": "working",
            "phase_changed_at": "2030-01-01T00:00:00Z",
            "current_turn": {"last_progress_at": "2030-01-01T00:04:00Z", "attention": null},
            "last_turn": null,
            "source": {"confidence": "authoritative"}
        },
        "state": "live",
        "runtime_status": "running",
        "created_at": "2030-01-01T00:00:00Z",
        "updated_at": "2030-01-01T00:04:00Z",
        "closed_at": null,
        "close_reason": null,
        "summary": null
    })
}

/// An aggregator view with the caller's own session, one messageable peer,
/// one peer that cannot receive, and one unavailable machine.
fn view() -> Value {
    let mut own = record(MACHINE, SESSION, Some(INCARNATION), true);
    own["console_owner"] = json!("owner-a");
    let mut peer = record(
        "host-b",
        "20300101-000000-peer",
        Some("peer-incarnation"),
        true,
    );
    peer["console_owner"] = json!(null);
    let quiet = record(
        "host-b",
        "20300101-000000-quiet",
        Some("quiet-incarnation"),
        false,
    );
    // The caller's session id on another machine is not the caller.
    let namesake = record("host-b", SESSION, Some("namesake-incarnation"), true);
    json!({
        "schema_version": "agent-session.board-view.v1",
        "record_schema": "agent-session.board-record.v1",
        "generated_at": "2030-01-01T00:05:00Z",
        "retention": "3d",
        "effective_since": "2029-12-29T00:05:00Z",
        "since_capped": true,
        "machines": [
            {"machine": MACHINE, "available": true, "last_seen_at": "2030-01-01T00:04:55Z"},
            {"machine": "host-b", "available": true, "last_seen_at": "2030-01-01T00:04:50Z"},
            {"machine": "host-c", "available": false, "last_seen_at": null}
        ],
        "records": [own, peer, quiet, namesake],
        "truncated": false
    })
}

fn error_of(output: &CmdOutput) -> Value {
    let body = output.stdout_json();
    assert_eq!(body["ok"], false, "{body}");
    assert!(body.get("data").is_none(), "no view on failure: {body}");
    body["error"].clone()
}

#[test]
fn relay_mode_forwards_filters_and_passes_the_view_through() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&["--board"], Some(&aggregator));
    aggregator.reply_json(200, &view());

    let output = fixture.board(&[
        "--state",
        "live",
        "--since",
        "2w",
        "--repo",
        "nils-cli",
        "--machine",
        "host-b",
        "--format",
        "json",
    ]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let body = output.stdout_json();
    assert_eq!(body["schema_version"], "cli.agent-session.board.v1");
    assert_eq!(body["data"]["mode"], "relay");
    // The view is the aggregator's, unchanged: since_capped, the unavailable
    // machine, and the display-only console_owner annotation included.
    assert_eq!(body["data"]["board"], view());

    let seen = aggregator.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].path(), "/api/coordination/board/v1");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some(format!("Bearer {RELAY_TOKEN}").as_str())
    );
    let mut query = seen[0].query();
    query.sort();
    assert_eq!(
        query,
        [
            ("machine", "host-b"),
            ("repo", "nils-cli"),
            ("since", "2w"),
            ("source_incarnation", INCARNATION),
            ("source_session_id", SESSION),
            ("state", "live"),
        ]
        .map(|(key, value)| (key.to_string(), value.to_string()))
    );

    // An omitted --since asks for the full retained window: no since at all.
    aggregator.reply_json(200, &view());
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let seen = aggregator.seen();
    let keys: Vec<String> = seen[1].query().into_iter().map(|(key, _)| key).collect();
    assert_eq!(
        keys,
        ["state", "source_session_id", "source_incarnation"].map(String::from)
    );
    assert_eq!(seen[1].query()[0].1, "all");
}

#[test]
fn relay_text_marks_the_caller_and_offers_send_targets_only_to_messageable_peers() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&["--board"], Some(&aggregator));
    let mut board = view();
    board["records"]
        .as_array_mut()
        .expect("records")
        .push(record(
            MACHINE,
            "20300101-000000-local",
            Some("local-incarnation"),
            true,
        ));
    aggregator.reply_json(200, &board);

    let output = fixture.board(&[]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let text = output.stdout_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "mode: relay",
            "unavailable  host-c  last seen -",
            "live  host-a  20300101-000000-self  nils-cli  working  1m  Title 20300101-000000-self  (this session)",
            "live  host-b  20300101-000000-peer  nils-cli  working  1m  Title 20300101-000000-peer  send: --to-machine host-b --to 20300101-000000-peer (incarnation peer-incarnation)",
            "live  host-b  20300101-000000-quiet  nils-cli  working  1m  Title 20300101-000000-quiet",
            "live  host-b  20300101-000000-self  nils-cli  working  1m  Title 20300101-000000-self  send: --to-machine host-b --to 20300101-000000-self (incarnation namesake-incarnation)",
            "live  host-a  20300101-000000-local  nils-cli  working  1m  Title 20300101-000000-local  send: --to 20300101-000000-local (incarnation local-incarnation)",
        ]
    );
}

#[test]
fn aggregator_query_invalid_is_forwarded_with_its_bounded_message() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&["--board"], Some(&aggregator));

    aggregator.reply_json(
        400,
        &json!({"error": {"code": "board-query-invalid", "message": "repo must not be empty"}}),
    );
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 64, "stdout={}", output.stdout_text());
    let error = error_of(&output);
    assert_eq!(error["code"], "board-query-invalid");
    assert_eq!(error["message"], "repo must not be empty");

    // A message that is not a bounded single line is replaced, not forwarded.
    for message in [json!("first\nsecond"), json!("x".repeat(4096)), json!(7)] {
        aggregator.reply_json(
            400,
            &json!({"error": {"code": "board-query-invalid", "message": message}}),
        );
        let output = fixture.board(&["--format", "json"]);
        assert_eq!(output.code, 64, "stdout={}", output.stdout_text());
        let error = error_of(&output);
        assert_eq!(error["code"], "board-query-invalid");
        assert_eq!(error["message"], "the aggregator rejected the board query");
    }
}

#[test]
fn relay_failures_map_to_stable_codes_and_never_fall_back_to_local() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&["--board"], Some(&aggregator));

    let unauthorized = json!({"error": {"code": "unauthorized"}});
    let cases = [
        (
            raw(401, &unauthorized.to_string(), ""),
            "board-relay-unauthorized",
        ),
        (
            raw(403, &unauthorized.to_string(), ""),
            "board-relay-unauthorized",
        ),
        (
            raw(503, r#"{"error":{"code":"machine-unavailable"}}"#, ""),
            "board-relay-unavailable",
        ),
        (
            raw(400, r#"{"error":{"code":"retention-exceeded"}}"#, ""),
            "board-relay-unavailable",
        ),
        (raw(502, "not json", ""), "board-relay-unavailable"),
        (
            raw(
                302,
                "{}",
                &format!("Location: {}/api/coordination/board/v1\r\n", aggregator.url),
            ),
            "board-relay-unavailable",
        ),
        (
            raw(
                200,
                r#"{"schema_version":"agent-session.board-view.v2"}"#,
                "",
            ),
            "board-relay-unavailable",
        ),
        (raw(200, "[]", ""), "board-relay-unavailable"),
    ];
    for (index, (response, code)) in cases.into_iter().enumerate() {
        aggregator.reply(response.clone());
        let output = fixture.board(&["--format", "json"]);
        assert_eq!(
            output.code,
            1,
            "{response}: stdout={}",
            output.stdout_text()
        );
        let error = error_of(&output);
        assert_eq!(error["code"], code, "{response}");
        // One aggregator call per run: a redirect is refused, not followed.
        assert_eq!(aggregator.seen().len(), index + 1, "{response}");
    }
}

#[test]
fn aggregator_scope_refusals_keep_their_own_codes() {
    // A principal-scoped aggregator refuses a caller its access store cannot
    // attribute. Those refusals are not credential failures: each keeps its
    // code through the daemon route and the CLI, with a data exit code.
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&["--board"], Some(&aggregator));
    let route = format!("/sessions/{SESSION}/board/v1");
    fixture.heartbeat();

    let cases = [
        (403, "ownership-unknown", 422),
        (403, "machine-forbidden", 422),
        (409, "session-incarnation-conflict", 409),
    ];
    for (status, code, daemon_status) in cases {
        let failure =
            json!({"ok": false, "error": {"code": code, "message": "refused by the aggregator"}});
        aggregator.reply_json(status, &failure);
        let (seen_status, body) = serve.get(&route, Some(CAPABILITY));
        assert_eq!(seen_status, daemon_status, "{code}: {body}");
        assert_eq!(body["error"]["code"], code, "{body}");

        aggregator.reply_json(status, &failure);
        let output = fixture.board(&["--format", "json"]);
        assert_eq!(output.code, 65, "{code}: stdout={}", output.stdout_text());
        let error = error_of(&output);
        assert_eq!(error["code"], code);
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "{error}"
        );
    }

    // Any other 401 or 403 is still the relay credential's rejection, and a
    // scope code on an unexpected status is not trusted.
    for (status, code) in [
        (403, "principal-forbidden"),
        (401, "ownership-unknown"),
        (403, "session-incarnation-conflict"),
    ] {
        aggregator.reply_json(status, &json!({"error": {"code": code}}));
        let output = fixture.board(&["--format", "json"]);
        assert_eq!(
            output.code,
            1,
            "{status} {code}: stdout={}",
            output.stdout_text()
        );
        assert_eq!(error_of(&output)["code"], "board-relay-unauthorized");
    }
    aggregator.reply_json(409, &json!({"error": {"code": "ownership-unknown"}}));
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 1, "stdout={}", output.stdout_text());
    assert_eq!(error_of(&output)["code"], "board-relay-unavailable");
}

#[test]
fn relay_disabled_or_board_disabled_daemon_selects_local_mode() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();

    // Board on, federation unconfigured: board-relay-disabled, no network.
    let serve = fixture.serve(&["--board"], None);
    let (status, body) = serve.get(&format!("/sessions/{SESSION}/board/v1"), Some(CAPABILITY));
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "board-relay-disabled");
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let body = output.stdout_json();
    assert_eq!(body["data"]["mode"], "local");
    assert_eq!(body["data"]["board"]["machines"][0]["machine"], MACHINE);
    drop(serve);

    // Board off: board-disabled before authentication.
    let serve = fixture.serve(&[], Some(&aggregator));
    let (status, body) = serve.get(&format!("/sessions/{SESSION}/board/v1"), None);
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "board-disabled");
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["mode"], "local");
    drop(serve);

    assert_eq!(aggregator.seen().len(), 0);
}

#[test]
fn relay_route_requires_the_current_session_capability_and_known_filters() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&["--board"], Some(&aggregator));
    let route = format!("/sessions/{SESSION}/board/v1");
    fixture.heartbeat();

    for bearer in [None, Some(OPERATOR), Some(RELAY_TOKEN)] {
        let (status, body) = serve.get(&route, bearer);
        assert_eq!(status, 401, "{bearer:?}: {body}");
        assert_eq!(body["error"]["code"], "coordination-unauthorized");
    }
    for query in [
        "?limit=5",
        "?state=live&state=closed",
        "?source_session_id=x",
    ] {
        let (status, body) = serve.get(&format!("{route}{query}"), Some(CAPABILITY));
        assert_eq!(status, 400, "{query}: {body}");
        assert_eq!(body["error"]["code"], "board-query-invalid");
    }
    assert_eq!(aggregator.seen().len(), 0);

    aggregator.reply_json(200, &view());
    let (status, body) = serve.get(&format!("{route}?state=closed"), Some(CAPABILITY));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, view());
    assert_eq!(
        aggregator.seen()[0].query()[0],
        ("state".into(), "closed".into())
    );
}

#[test]
fn a_claimed_identity_that_fails_authentication_is_an_error_not_a_local_view() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&["--board"], Some(&aggregator));
    let heartbeat = fixture
        .state_dir
        .join("sessions")
        .join(SESSION)
        .join("coordination/heartbeat");
    let capability = fs::read(&fixture.capability_file).expect("capability");

    // A rotated capability no longer matches the broker.
    private_file(
        &fixture.capability_file,
        b"board-relay-rotated-capability-00000000000001",
    );
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(error_of(&output)["code"], "coordination-unauthorized");
    private_file(&fixture.capability_file, &capability);

    // A stale broker heartbeat: the broker is lost.
    let run_stale = || {
        let state = fixture.state_dir.to_string_lossy().to_string();
        let capability = fixture.capability_file.to_string_lossy().to_string();
        private_file(
            &heartbeat,
            format!("{INCARNATION}:{}\n", now_epoch() - 600).as_bytes(),
        );
        let options = CmdOptions::new()
            .with_cwd(&fixture.root)
            .without_ambient_managed_session_env()
            .with_env_remove_many(&ISOLATED_ENV)
            .with_envs(&[
                ("AGENT_SESSION_MACHINE", MACHINE),
                ("AGENT_SESSION_ID", SESSION),
                ("AGENT_SESSION_CAPABILITY_FILE", capability.as_str()),
            ]);
        run_resolved(
            "agent-session",
            &["--state-dir", state.as_str(), "board", "--format", "json"],
            &options,
        )
    };
    let output = run_stale();
    assert_eq!(output.code, 1, "stdout={}", output.stdout_text());
    assert_eq!(error_of(&output)["code"], "coordination-broker-lost");
    assert_eq!(aggregator.seen().len(), 0);

    // Without a daemon endpoint there is no relay to fail: local mode.
    fs::remove_file(fixture.state_dir.join("coordination/daemon-endpoint.json"))
        .expect("drop endpoint");
    let output = run_stale();
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["mode"], "local");
}

#[test]
fn an_unreachable_daemon_is_an_error_not_a_local_view() {
    let fixture = Fixture::new();
    // A daemon endpoint that no longer answers, as after serve stopped.
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("reserve port")
        .local_addr()
        .expect("addr")
        .port();
    private_file(
        &fixture.state_dir.join("coordination/daemon-endpoint.json"),
        json!({"url": format!("http://127.0.0.1:{port}")})
            .to_string()
            .as_bytes(),
    );
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 1, "stdout={}", output.stdout_text());
    assert_eq!(error_of(&output)["code"], "board-relay-unavailable");

    // Without the endpoint file there is no relay to try: local mode.
    fs::remove_file(fixture.state_dir.join("coordination/daemon-endpoint.json"))
        .expect("drop endpoint");
    let output = fixture.board(&["--format", "json"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["mode"], "local");
}

#[test]
fn console_start_creates_the_child_through_the_aggregator_as_the_calling_session() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&[], Some(&aggregator));
    let child = json!({
        "id": "0b6f4d7e-1c11-4a4f-9c1e-3f4d8f0c2a55",
        "agent": "claude",
        "cwd": "/work/repo",
        "session_incarnation": "child-incarnation",
        "status": "running",
    });
    aggregator.reply_json(201, &json!({"ok": true, "data": {"session": child}}));

    let output = fixture.start_via_console(
        &[
            "--agent",
            "claude",
            "--cwd",
            "/work/repo",
            "--title",
            "Child task",
            "--prompt",
            "do the thing",
            "--agent-arg=--verbose",
            "--machine",
            "host-b",
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let body = output.stdout_json();
    assert_eq!(body["schema_version"], "cli.agent-session.console-start.v1");
    assert_eq!(
        body["data"],
        json!({"schema_version": "agent-session.console-start.v1", "machine": "host-b", "session": child})
    );

    let seen = aggregator.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].path(), "/api/coordination/sessions/v1");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some(format!("Bearer {RELAY_TOKEN}").as_str())
    );
    // The daemon adds the caller's exact identity; nothing names an owner.
    assert_eq!(
        seen[0].body,
        Some(json!({
            "source_session_id": SESSION,
            "source_incarnation": INCARNATION,
            "machine": "host-b",
            "session": {
                "agent": "claude", "cwd": "/work/repo", "title": "Child task",
                "prompt": "do the thing", "agent_args": ["--verbose"],
                "lineage": child_lineage()
            }
        }))
    );

    // Without --machine the child starts on the daemon's own machine.
    aggregator.reply_json(201, &json!({"ok": true, "data": {"session": child}}));
    let output = fixture.start_via_console(&["--agent", "claude", "--cwd", "/work/repo"], true);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(
        output.stdout_text(),
        format!(
            "started claude session {} on {MACHINE} through Agent Console\n",
            child["id"].as_str().unwrap()
        )
    );
    let seen = aggregator.seen();
    assert_eq!(
        seen[1]
            .body
            .as_ref()
            .map(|body| body.get("machine").is_none()),
        Some(true)
    );
}

#[test]
fn console_start_sends_the_child_lineage_and_resolved_work() {
    let fixture = Fixture::new();
    let record_path = fixture
        .state_dir
        .join("sessions")
        .join(SESSION)
        .join("session.json");
    let mut record: Value =
        serde_json::from_slice(&fs::read(&record_path).expect("record")).expect("record json");
    let root = json!({
        "machine": "sympoies",
        "session_id": "laoda-root",
        "session_created_at": "2029-12-31T00:00:00Z",
    });
    record["lineage"] = json!({
        "schema_version": "agent-session.session-lineage.v1",
        "parent": root,
        "root": root,
        "depth": 1,
        "starter": {"kind": "session", "via": "console"},
        "budget": null,
    });
    let program = json!({"provider": "github", "repository": "serenvia/laoda", "number": 44});
    let issue = json!({"provider": "github", "repository": "sympoies/nils-cli", "number": 2032});
    record["work"] =
        json!({"program": program, "issues": [issue], "inherited": false, "revision": 2});
    private_file(&record_path, &serde_json::to_vec(&record).expect("record"));
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&[], Some(&aggregator));
    let created = json!({"ok": true, "data": {"session": {"id": "child", "agent": "claude"}}});
    let parent = json!({
        "machine": MACHINE,
        "session_id": SESSION,
        "session_created_at": "2030-01-01T00:00:00Z",
        "session_incarnation": INCARNATION,
    });
    let session_of = |seen: &[Seen]| {
        seen.last()
            .and_then(|seen| seen.body.clone())
            .expect("body")["session"]
            .clone()
    };

    // The child keeps the caller's root one level deeper and inherits work.
    aggregator.reply_json(201, &created);
    let output = fixture.start_via_console(&["--agent", "claude", "--cwd", "/w"], true);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let session = session_of(&aggregator.seen());
    assert_eq!(
        session["lineage"],
        json!({
            "schema_version": "agent-session.session-lineage.v1",
            "parent": parent,
            "root": root,
            "depth": 2,
            "starter": {"kind": "session", "via": "console"},
        })
    );
    assert_eq!(
        session["work"],
        json!({"program": program, "issues": [issue], "inherited": true})
    );

    // An explicit issue replaces only the issues.
    aggregator.reply_json(201, &created);
    let output = fixture.start_via_console(
        &[
            "--agent",
            "claude",
            "--cwd",
            "/w",
            "--issue",
            "sympoies/nils-cli#2040",
        ],
        true,
    );
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let session = session_of(&aggregator.seen());
    assert_eq!(
        session["work"],
        json!({
            "program": program,
            "issues": [{"provider": "github", "repository": "sympoies/nils-cli", "number": 2040}],
            "inherited": false
        })
    );

    // --no-parent asks for a new root: no parent, and no inherited work.
    aggregator.reply_json(201, &created);
    let output =
        fixture.start_via_console(&["--agent", "claude", "--cwd", "/w", "--no-parent"], true);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let session = session_of(&aggregator.seen());
    assert_eq!(
        session["lineage"],
        json!({
            "schema_version": "agent-session.session-lineage.v1",
            "parent": null,
            "root": null,
            "depth": 0,
            "starter": {"kind": "operator", "via": "console"},
        })
    );
    assert!(session.get("work").is_none(), "{session}");
}

#[test]
fn a_managed_session_sets_only_its_own_work_and_adopts_only_for_itself() {
    let fixture = Fixture::new();
    let session_dir = fixture.state_dir.join("sessions");
    let mut child: Value = serde_json::from_slice(
        &fs::read(session_dir.join(SESSION).join("session.json")).expect("record"),
    )
    .expect("record json");
    child["id"] = json!("20300101-000000-child");
    child["tmux_session"] = json!("agent-child");
    child["runtime"]["tmux_session"] = json!("agent-child");
    child["runtime"]["launch_id"] = json!("child-incarnation");
    private_dir(&session_dir.join("20300101-000000-child"));
    private_file(
        &session_dir.join("20300101-000000-child/session.json"),
        &serde_json::to_vec(&child).expect("child"),
    );
    let code = |output: &CmdOutput| output.stdout_json()["error"]["code"].clone();

    let output = fixture.run(
        "work",
        &[
            "set",
            SESSION,
            "--issue",
            "sympoies/nils-cli#2032",
            "--if-revision",
            "0",
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["work"]["revision"], 1);
    let output = fixture.run(
        "work",
        &[
            "set",
            "20300101-000000-child",
            "--issue",
            "sympoies/nils-cli#2032",
            "--if-revision",
            "0",
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(code(&output), "work-set-forbidden");

    let output = fixture.run(
        "lineage",
        &[
            "adopt",
            "20300101-000000-child",
            "--by",
            SESSION,
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["lineage_adoption"]["adopted_by"],
        json!({
            "machine": MACHINE,
            "session_id": SESSION,
            "session_created_at": "2030-01-01T00:00:00Z",
            "session_incarnation": INCARNATION,
        })
    );
    for args in [
        vec![
            "adopt",
            SESSION,
            "--by",
            "20300101-000000-child",
            "--format",
            "json",
        ],
        vec![
            "adopt",
            "20300101-000000-child",
            "--clear",
            "--format",
            "json",
        ],
        vec![
            "adopt",
            "20300101-000000-child",
            "--by",
            SESSION,
            "--by-machine",
            "other-host",
            "--by-created-at",
            "2030-01-01T00:00:00Z",
            "--format",
            "json",
        ],
    ] {
        let output = fixture.run("lineage", &args, true);
        assert_eq!(output.code, 65, "{args:?}: stdout={}", output.stdout_text());
        assert_eq!(code(&output), "lineage-adopt-forbidden", "{args:?}");
    }

    // Without its capability a managed session is refused, not an operator.
    fs::remove_file(&fixture.capability_file).expect("remove capability");
    let output = fixture.run(
        "work",
        &[
            "set",
            SESSION,
            "--clear-issues",
            "--if-revision",
            "1",
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(code(&output), "coordination-unauthorized");
}

#[test]
fn console_start_forwards_ownership_refusals_with_fixed_classes() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&[], Some(&aggregator));
    for (status, code) in [
        (403, "ownership-unknown"),
        (403, "machine-forbidden"),
        (409, "session-incarnation-conflict"),
    ] {
        aggregator.reply_json(
            status,
            &json!({"ok": false, "error": {"code": code, "message": "refused by the console"}}),
        );
        let output = fixture.start_via_console(&["--agent", "codex", "--format", "json"], true);
        assert_eq!(output.code, 65, "{code}: stdout={}", output.stdout_text());
        let error = error_of(&output);
        assert_eq!(error["code"], code);
        assert_eq!(error["message"], "refused by the console");
    }
    // An aggregator that rejects the relay credential is unavailable, not an
    // ownership answer.
    aggregator.reply_json(
        401,
        &json!({"ok": false, "error": {"code": "unauthorized", "message": "x"}}),
    );
    let output = fixture.start_via_console(&["--agent", "codex", "--format", "json"], true);
    assert_eq!(error_of(&output)["code"], "console-start-unavailable");
}

#[test]
fn console_start_needs_federation_and_a_managed_session() {
    let fixture = Fixture::new();
    let _serve = fixture.serve(&[], None);
    let output = fixture.start_via_console(&["--agent", "claude", "--format", "json"], true);
    assert_eq!(output.code, 1, "stdout={}", output.stdout_text());
    assert_eq!(error_of(&output)["code"], "console-start-disabled");

    let output = fixture.start_via_console(&["--agent", "claude", "--format", "json"], false);
    assert_eq!(output.code, 64, "stdout={}", output.stdout_text());
    assert_eq!(error_of(&output)["code"], "console-start-unmanaged");

    // --machine only means something through the console, and a console
    // start cannot pick the session id or the local launch.
    for args in [
        vec![
            "start",
            "--via-console",
            "--agent",
            "claude",
            "--coordination-mode",
            "enforce",
        ],
        vec!["start", "--agent", "claude", "--machine", "host-b"],
        vec![
            "start",
            "--via-console",
            "--agent",
            "claude",
            "--id",
            "chosen",
        ],
        vec![
            "start",
            "--via-console",
            "--agent",
            "claude",
            "--agent-bin",
            "/bin/true",
        ],
    ] {
        let output = fixture.run(args[0], &args[1..], true);
        assert_eq!(output.code, 64, "{args:?}: stdout={}", output.stdout_text());
    }

    // A managed id whose capability is gone is an authentication failure.
    fs::remove_file(&fixture.capability_file).expect("remove capability");
    let output = fixture.start_via_console(&["--agent", "claude", "--format", "json"], true);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(error_of(&output)["code"], "coordination-unauthorized");
}

#[test]
fn console_start_route_requires_the_current_session_capability_and_a_known_body() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    let post = |bearer: &str, body: Value| {
        let response = reqwest::blocking::Client::new()
            .post(format!(
                "http://{}/sessions/{SESSION}/console-start/v1",
                serve.addr
            ))
            .bearer_auth(bearer)
            .json(&body)
            .send()
            .expect("serve request");
        let status = response.status().as_u16();
        (status, response.json::<Value>().expect("json body"))
    };
    fixture.heartbeat();
    let (status, body) = post(OPERATOR, json!({"session": {"agent": "claude"}}));
    assert_eq!(
        (status, &body["error"]["code"]),
        (401, &json!("coordination-unauthorized"))
    );
    for invalid in [
        json!({"session": {"agent": "claude"}, "principal": "someone-else"}),
        json!({"machine": "host-b"}),
        json!({"session": {"agent": "claude"}, "no_parent": "yes"}),
        json!({"session": {"agent": "claude"}, "work": {"issues": ["free text"]}}),
    ] {
        let (status, body) = post(CAPABILITY, invalid);
        assert_eq!(
            (status, &body["error"]["code"]),
            (400, &json!("console-start-invalid"))
        );
    }
    assert!(aggregator.seen().is_empty(), "{:?}", aggregator.seen());
}

#[test]
fn console_start_selects_the_account_for_its_agent_and_the_launch_profile() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&[], Some(&aggregator));
    let created = json!({"ok": true, "data": {"session": {"id": "child", "agent": "codex"}}});
    for (agent, field) in [("codex", "codex_account"), ("claude", "claude_account")] {
        aggregator.reply_json(201, &created);
        let output = fixture.start_via_console(
            &[
                "--agent",
                agent,
                "--cwd",
                "/w",
                "--account",
                "spare",
                "--format",
                "json",
            ],
            true,
        );
        assert_eq!(output.code, 0, "{agent}: stdout={}", output.stdout_text());
        let seen = aggregator.seen();
        let session = &seen
            .last()
            .and_then(|seen| seen.body.clone())
            .expect("body")["session"];
        assert_eq!(
            session,
            &json!({"agent": agent, "cwd": "/w", field: "spare", "lineage": child_lineage()})
        );
    }
    aggregator.reply_json(201, &created);
    let output = fixture.start_via_console(
        &[
            "--agent",
            "claude",
            "--cwd",
            "/w",
            "--agent-profile",
            "claude-opus",
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let seen = aggregator.seen();
    assert_eq!(
        seen.last()
            .and_then(|seen| seen.body.clone())
            .expect("body")["session"]["agent_profile"],
        "claude-opus"
    );

    let requests = aggregator.seen().len();
    let output = fixture.start_via_console(
        &[
            "--agent",
            "hermes",
            "--account",
            "spare",
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 64, "stdout={}", output.stdout_text());
    assert_eq!(
        error_of(&output)["code"],
        "console-start-account-unsupported"
    );
    assert_eq!(aggregator.seen().len(), requests);
}
