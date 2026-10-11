//! Black-box contract for the loopback activity hook ingress.
//!
//! A provider hook normally reports lifecycle metadata by writing the session
//! state directory (`agent-session activity hook`). `activity hook --via http`
//! reports the identical payload to `POST /activity/hook/v1` on the local
//! `agent-session serve` daemon instead, authenticated by the session's
//! per-incarnation capability. These tests pin that both transports produce the
//! same durable `turn_state` transition and that a credential for another
//! session or incarnation never mutates state.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nils_test_support::bin;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const TOKEN: &str = "activity-hook-ingress-operator-bearer";
const MACHINE: &str = "activity-hook-ingress-machine";
const ALPHA: &str = "hook-ingress-alpha";
const BETA: &str = "hook-ingress-beta";
const ALPHA_CAPABILITY: &str = "alpha-activity-hook-ingress-capability-material-0001";
const BETA_CAPABILITY: &str = "beta-activity-hook-ingress-capability-material-00002";
const PROMPT_CANARY: &str = "activity-hook-ingress-prompt-canary";

fn user_prompt_submit() -> String {
    json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": "provider-session-0001",
        "prompt": PROMPT_CANARY,
    })
    .to_string()
}

fn incarnation(id: &str) -> String {
    format!("launch-{id}")
}

#[test]
fn http_hook_event_produces_the_same_turn_state_transition_as_the_file_path() {
    let fixture = Fixture::new();
    let server = ServeProcess::spawn(&fixture);

    assert_eq!(fixture.turn_state(ALPHA)["phase"], "starting");
    assert_eq!(fixture.turn_state(BETA)["phase"], "starting");

    let file_path = fixture.hook(ALPHA, ALPHA, None, &user_prompt_submit());
    assert!(file_path.status.success(), "{}", file_path.stderr);
    let http_path = fixture.hook(BETA, BETA, Some("http"), &user_prompt_submit());
    assert!(http_path.status.success(), "{}", http_path.stderr);
    assert_eq!(http_path.stdout, "", "a hook stays silent on success");

    let alpha = fixture.turn_state(ALPHA);
    let beta = fixture.turn_state(BETA);
    assert_eq!(alpha["phase"], "working", "file path transition: {alpha}");
    assert_eq!(
        comparable(&beta),
        comparable(&alpha),
        "HTTP ingress must produce the file path's turn_state"
    );
    assert!(
        !server.stderr().contains(ALPHA_CAPABILITY)
            && !server.stderr().contains(BETA_CAPABILITY)
            && !server.stderr().contains(PROMPT_CANARY),
        "the daemon must not log credentials or hook content"
    );
}

#[test]
fn auth_loss_real_hook_notifies_owner_once_and_reports_recovery() {
    for discovery_available in [true, false] {
        let fixture = Fixture::new();
        if !discovery_available {
            write_executable(&fixture.tmux_bin, "#!/bin/sh\nexit 1\n");
        }
        let record_path = fixture
            .state_dir
            .join("sessions")
            .join(ALPHA)
            .join("session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        let parent = json!({"machine":MACHINE, "session_id":BETA, "session_created_at":"2030-01-01T00:00:00Z"});
        record["lineage"] = json!({"schema_version":"agent-session.session-lineage.v1", "machine":MACHINE, "parent":parent, "root":parent, "depth":1, "starter":{"kind":"session", "via":"cli"}, "budget":null});
        write_private(&record_path, &serde_json::to_vec(&record).unwrap());
        let server = ServeProcess::spawn(&fixture);
        let payload = json!({"hook_event_name":"StopFailure", "session_id":"provider-session-0001", "error":"authentication_failed", "error_details":PROMPT_CANARY}).to_string();
        let started = Instant::now();
        for _ in 0..3 {
            let result = fixture.hook(ALPHA, ALPHA, Some("http"), &payload);
            assert!(result.status.success(), "{}", result.stderr);
        }
        assert_eq!(
            fixture.turn_state(ALPHA)["last_turn"]["provider_failure_kind"],
            "authentication"
        );
        let deadline = started + Duration::from_secs(60);
        loop {
            let data = fixture.inbox(BETA);
            let count = data["data"]["messages"].as_array().unwrap().len();
            if count == 1 {
                break;
            }
            assert_eq!(count, 0, "one deduped owner notification");
            assert!(
                Instant::now() < deadline,
                "owner notification exceeded 60 seconds"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let completed = json!({"hook_event_name":"Notification", "notification_type":"idle_prompt", "session_id":"provider-session-0001"}).to_string();
        assert!(
            fixture
                .hook(ALPHA, ALPHA, Some("http"), &completed)
                .status
                .success()
        );
        let recovery_deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let data = fixture.inbox(BETA);
            let messages = data["data"]["messages"].as_array().unwrap();
            if messages.len() == 2 {
                break;
            }
            assert_eq!(messages.len(), 1);
            assert!(
                Instant::now() < recovery_deadline,
                "missing recovery-result notification"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            fixture
                .hook(ALPHA, ALPHA, Some("http"), &completed)
                .status
                .success()
        );
        assert_eq!(
            fixture.inbox(BETA)["data"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let incident_path = fixture
            .state_dir
            .join("sessions")
            .join(ALPHA)
            .join("auth-incidents.json");
        let incidents = fs::read_to_string(incident_path).unwrap();
        let stored: Value = serde_json::from_str(&incidents).unwrap();
        assert_eq!(stored["incidents"].as_array().unwrap().len(), 1);
        assert_eq!(stored["incidents"][0]["status"], "recovered");
        assert_eq!(stored["incidents"][0]["recovery_result"], "healthy");
        assert!(stored["incidents"][0]["recovery_notification"]["delivered_at"].is_string());
        assert!(!incidents.contains(PROMPT_CANARY));
        assert!(!server.stderr().contains(PROMPT_CANARY));
    }
}

#[test]
fn http_hook_rejects_a_credential_for_another_session_or_incarnation() {
    let fixture = Fixture::new();
    let server = ServeProcess::spawn(&fixture);

    // The CLI stays fail-open: a mismatched credential exits 0 without a
    // transition, exactly like an invalid file-path hook.
    let cross_session = fixture.hook(BETA, ALPHA, Some("http"), &user_prompt_submit());
    assert!(cross_session.status.success(), "{}", cross_session.stderr);
    let untouched = fixture.turn_state(BETA);
    assert_eq!(untouched["phase"], "starting");
    assert_eq!(fixture.turn_state(ALPHA)["phase"], "starting");

    let body = |session: &str, session_incarnation: &str| {
        json!({
            "schema_version": "agent-session.activity-hook.v1",
            "session_id": session,
            "session_incarnation": session_incarnation,
            "agent": "claude",
            "payload": user_prompt_submit(),
        })
    };
    for (name, capability, request) in [
        (
            "another session's capability",
            Some(ALPHA_CAPABILITY),
            body(BETA, &incarnation(BETA)),
        ),
        (
            "a stale incarnation",
            Some(BETA_CAPABILITY),
            body(BETA, "launch-replaced-incarnation"),
        ),
        (
            "an unknown capability",
            Some("unknown-activity-hook-capability-material-000000001"),
            body(BETA, &incarnation(BETA)),
        ),
        ("a missing capability", None, body(BETA, &incarnation(BETA))),
    ] {
        let response = server.post_hook(capability, &request);
        assert_eq!(response.status, 401, "{name}: {}", response.body);
        assert_eq!(
            response.body["error"]["code"], "coordination-unauthorized",
            "{name}"
        );
        let text = response.body.to_string();
        assert!(
            !text.contains(ALPHA_CAPABILITY) && !text.contains(BETA_CAPABILITY),
            "{name}: rejection must not echo credential material"
        );
    }
    assert_eq!(fixture.turn_state(BETA), untouched);

    let accepted = server.post_hook(Some(BETA_CAPABILITY), &body(BETA, &incarnation(BETA)));
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(accepted.body["data"]["ingested"], true);
    assert_eq!(fixture.turn_state(BETA)["phase"], "working");
}

#[test]
fn http_hook_rejects_requests_outside_the_hook_event_schema() {
    let fixture = Fixture::new();
    let server = ServeProcess::spawn(&fixture);
    let valid = json!({
        "schema_version": "agent-session.activity-hook.v1",
        "session_id": BETA,
        "session_incarnation": incarnation(BETA),
        "agent": "claude",
        "payload": user_prompt_submit(),
    });

    let mut unknown_field = valid.clone();
    unknown_field["turn_state"] = json!({"phase": "waiting"});
    let mut wrong_schema = valid.clone();
    wrong_schema["schema_version"] = json!("agent-session.activity-hook.v0");
    let mut unknown_agent = valid.clone();
    unknown_agent["agent"] = json!("unknown-agent");
    for (name, request) in [
        ("unknown field", unknown_field),
        ("wrong schema", wrong_schema),
        ("unknown agent", unknown_agent),
    ] {
        let response = server.post_hook(Some(BETA_CAPABILITY), &request);
        assert_eq!(response.status, 400, "{name}: {}", response.body);
    }

    // An oversized provider payload is refused with the file path's own code.
    let mut oversized = valid.clone();
    oversized["payload"] = json!(format!(
        "{{\"hook_event_name\":\"UserPromptSubmit\",\"pad\":\"{}\"}}",
        "x".repeat(64 * 1024)
    ));
    let response = server.post_hook(Some(BETA_CAPABILITY), &oversized);
    assert_eq!(response.status, 422, "{}", response.body);
    assert_eq!(response.body["error"]["code"], "provider-hook-too-large");
    assert_eq!(fixture.turn_state(BETA)["phase"], "starting");

    // The silent `--via http` client relies on the daemon recording the
    // ingestion failure exactly as the file path would.
    let diagnostic_path = fixture
        .state_dir
        .join("sessions")
        .join(BETA)
        .join("activity.diagnostic.json");
    let diagnostic: Value =
        serde_json::from_slice(&fs::read(&diagnostic_path).expect("activity diagnostic"))
            .expect("activity diagnostic JSON");
    assert_eq!(diagnostic["code"], "provider-hook-too-large");
    assert_eq!(diagnostic["runtime_id"], incarnation(BETA));
    let accepted = server.post_hook(Some(BETA_CAPABILITY), &valid);
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert!(
        !diagnostic_path.exists(),
        "a successful ingest clears the diagnostic"
    );
}

#[test]
fn http_hook_admits_a_burst_of_concurrent_hooks_from_one_session() {
    let fixture = Fixture::new();
    let server = ServeProcess::spawn(&fixture);
    let body = json!({
        "schema_version": "agent-session.activity-hook.v1",
        "session_id": BETA,
        "session_incarnation": incarnation(BETA),
        "agent": "claude",
        "payload": user_prompt_submit(),
    });
    // Parallel tool calls fire overlapping hooks; each must be ingested, as the
    // file path would after waiting on the session lock, rather than dropped.
    let statuses = std::thread::scope(|scope| {
        let handles = (0..12)
            .map(|_| scope.spawn(|| server.post_hook(Some(BETA_CAPABILITY), &body)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                let response = handle.join().expect("concurrent hook");
                (response.status, response.body.to_string())
            })
            .collect::<Vec<_>>()
    });
    for (status, body) in &statuses {
        assert_eq!(*status, 200, "{body}");
    }
    assert_eq!(fixture.turn_state(BETA)["phase"], "working");
}

/// Drop timestamps, which necessarily differ between two ingestions.
fn comparable(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !key.ends_with("_at"))
                .map(|(key, value)| (key.clone(), comparable(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(comparable).collect()),
        other => other.clone(),
    }
}

struct HookOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

struct HttpResponse {
    status: u16,
    body: Value,
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    state_dir: PathBuf,
    tmux_bin: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("activity hook ingress fixture");
        let root = tmp.path().to_path_buf();
        let home = root.join("home");
        let state_dir = root.join("state");
        let tmux_bin = root.join("tmux");
        fs::create_dir_all(&home).expect("fixture home");
        fs::create_dir_all(&state_dir).expect("fixture state");
        write_executable(
            &tmux_bin,
            "#!/bin/sh\ncase \"$1\" in list-sessions) printf 'hs-claude-hook-ingress-alpha\\nhs-claude-hook-ingress-beta\\n' ;; esac\nexit 0\n",
        );
        seed_brokers(
            &state_dir,
            &[(ALPHA, ALPHA_CAPABILITY), (BETA, BETA_CAPABILITY)],
        );
        Self {
            _tmp: tmp,
            root,
            home,
            state_dir,
            tmux_bin,
        }
    }

    fn capability_file(&self, id: &str) -> PathBuf {
        self.state_dir
            .join("sessions")
            .join(id)
            .join("coordination")
            .join(format!("capability-{}", digest(&incarnation(id))))
    }

    /// Run the provider hook as a managed runtime of `session` would, holding
    /// the capability file of `capability_owner`.
    fn hook(
        &self,
        session: &str,
        capability_owner: &str,
        via: Option<&str>,
        payload: &str,
    ) -> HookOutput {
        let mut command = Command::new(bin::resolve("agent-session"));
        command.args(["activity", "hook", "--agent", "claude"]);
        if let Some(via) = via {
            command.args(["--via", via]);
        }
        let mut child = command
            .current_dir(&self.root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("AGENT_SESSION_STATE_DIR", &self.state_dir)
            .env("AGENT_SESSION_ID", session)
            .env("AGENT_SESSION_RUNTIME_ID", incarnation(session))
            .env(
                "AGENT_SESSION_CAPABILITY_FILE",
                self.capability_file(capability_owner),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn activity hook");
        child
            .stdin
            .take()
            .expect("hook stdin")
            .write_all(payload.as_bytes())
            .expect("write hook payload");
        let output = child.wait_with_output().expect("activity hook output");
        HookOutput {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn inbox(&self, id: &str) -> Value {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        for session in [ALPHA, BETA] {
            write_private(
                &self
                    .state_dir
                    .join("sessions")
                    .join(session)
                    .join("coordination/heartbeat"),
                format!("{}:{now}\n", incarnation(session)).as_bytes(),
            );
        }
        let output = Command::new(bin::resolve("agent-session"))
            .args(["message", "inbox", "--session", id, "--format", "json"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("AGENT_SESSION_STATE_DIR", &self.state_dir)
            .env("AGENT_SESSION_CAPABILITY_FILE", self.capability_file(id))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn turn_state(&self, id: &str) -> Value {
        let output = Command::new(bin::resolve("agent-session"))
            .args(["activity", "status", id, "--format", "json"])
            .current_dir(&self.root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("AGENT_SESSION_STATE_DIR", &self.state_dir)
            .output()
            .expect("activity status");
        assert!(
            output.status.success(),
            "activity status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).expect("activity status JSON");
        value["data"]["turn_state"].clone()
    }
}

struct ServeProcess {
    child: Child,
    stderr_path: PathBuf,
    address: SocketAddr,
}

impl ServeProcess {
    fn spawn(fixture: &Fixture) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback address");
        let address = listener.local_addr().expect("loopback address");
        drop(listener);
        let stderr_path = fixture.root.join("serve.stderr");
        let stderr = File::create(&stderr_path).expect("serve stderr");
        let mut command = Command::new(bin::resolve("agent-session"));
        command
            .current_dir(&fixture.root)
            .args([
                "serve",
                "--bind",
                &address.to_string(),
                "--state-dir",
                fixture.state_dir.to_str().expect("UTF-8 state dir"),
                "--token",
                TOKEN,
                "--machine",
                MACHINE,
                "--tmux-bin",
                fixture.tmux_bin.to_str().expect("UTF-8 tmux fixture"),
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &fixture.home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        let mut server = Self {
            child: command.spawn().expect("spawn agent-session serve"),
            stderr_path,
            address,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let endpoint = fixture.state_dir.join("coordination/daemon-endpoint.json");
        while TcpStream::connect(address).is_err() || !endpoint.exists() {
            if let Some(status) = server.child.try_wait().expect("poll serve child") {
                panic!(
                    "agent-session serve exited before listening: status={status}; stderr={}",
                    server.stderr()
                );
            }
            assert!(
                Instant::now() < deadline,
                "agent-session serve did not listen; stderr={}",
                server.stderr()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        server
    }

    fn stderr(&self) -> String {
        fs::read_to_string(&self.stderr_path).unwrap_or_default()
    }

    fn post_hook(&self, capability: Option<&str>, body: &Value) -> HttpResponse {
        let mut stream = TcpStream::connect(self.address).expect("connect agent-session serve");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("HTTP read timeout");
        let body = body.to_string();
        let mut head = format!(
            "POST /activity/hook/v1 HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.address,
            body.len()
        );
        if let Some(capability) = capability {
            head.push_str(&format!("X-Agent-Session-Capability: {capability}\r\n"));
        }
        write!(stream, "{head}\r\n{body}").expect("HTTP request");
        stream.flush().expect("flush HTTP request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("read HTTP response");
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("HTTP response header boundary");
        let headers = std::str::from_utf8(&response[..header_end]).expect("HTTP headers");
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u16>().ok())
            .expect("HTTP status");
        let body = serde_json::from_slice(&response[header_end + 4..]).unwrap_or_else(
            |_| json!({"raw": String::from_utf8_lossy(&response[header_end + 4..])}),
        );
        HttpResponse { status, body }
    }
}

impl Drop for ServeProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn seed_brokers(state_dir: &Path, sessions: &[(&str, &str)]) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700)).expect("state mode");
    let mut brokers = serde_json::Map::new();
    for (id, capability) in sessions {
        let launch_id = incarnation(id);
        let session_dir = state_dir.join("sessions").join(id);
        fs::create_dir_all(&session_dir).expect("session directory");
        fs::set_permissions(
            state_dir.join("sessions"),
            fs::Permissions::from_mode(0o700),
        )
        .expect("sessions mode");
        fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o700)).expect("session mode");
        write_private(
            &session_dir.join("session.json"),
            &serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.session.v1",
                "id": id,
                "agent": "claude",
                "mode": "interactive",
                "title": "activity hook ingress fixture",
                "title_revision": 0,
                "cwd": "/fixture/repository",
                "tmux_session": format!("hs-claude-{id}"),
                "prompt_file": null,
                "log_file": null,
                "created_at": "2030-01-01T00:00:00Z",
                "updated_at": "2030-01-01T00:00:00Z",
                "runtime": {
                    "kind": "tmux",
                    "tmux_session": format!("hs-claude-{id}"),
                    "generation": 1,
                    "started_at": "2030-01-01T00:00:00Z",
                    "launch_id": launch_id
                }
            }))
            .expect("session json"),
        );
        write_private(
            &session_dir.join("activity.json"),
            &serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.activity.v1",
                "runtime_id": launch_id,
                "runtime_generation": 1,
                "state": {
                    "schema_version": "agent-session.turn-state.v1",
                    "phase": "starting",
                    "phase_changed_at": "2030-01-01T00:00:00Z",
                    "revision": 1,
                    "source": {
                        "kind": "runtime",
                        "provider": null,
                        "confidence": "authoritative"
                    },
                    "current_turn": null,
                    "last_turn": null
                },
                "pending_attention": [],
                "seen_event_count": 0
            }))
            .expect("activity json"),
        );
        let coordination_dir = session_dir.join("coordination");
        fs::create_dir(&coordination_dir).expect("coordination directory");
        fs::set_permissions(&coordination_dir, fs::Permissions::from_mode(0o700))
            .expect("coordination dir mode");
        write_private(
            &coordination_dir.join(format!("capability-{}", digest(&launch_id))),
            capability.as_bytes(),
        );
        write_private(
            &coordination_dir.join("heartbeat"),
            format!("{launch_id}:{now}\n").as_bytes(),
        );
        brokers.insert(
            (*id).to_string(),
            json!({
                "session_id": id,
                "incarnation": launch_id,
                "coordination_mode": "advisory",
                "capability_digest": digest(capability),
                "generation": 1,
                "state": "ready",
                "heartbeat_at": "2030-01-01T00:00:00Z",
                "heartbeat_epoch": now
            }),
        );
    }
    let coordination = state_dir.join("coordination");
    fs::create_dir(&coordination).expect("coordination root");
    fs::set_permissions(&coordination, fs::Permissions::from_mode(0o700))
        .expect("coordination mode");
    write_private(
        &coordination.join("registry.json"),
        &serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.coordination-registry.v1",
            "fingerprint_epoch": 1,
            "fingerprint_key": "fixture-private-fingerprint-key-material-0000000001",
            "brokers": brokers,
            "claims": [],
            "operations": [],
            "messages": [],
            "receipts": {},
            "notifications": {}
        }))
        .expect("registry json"),
    );
}

fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("write private fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("private fixture mode");
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable mode");
}
