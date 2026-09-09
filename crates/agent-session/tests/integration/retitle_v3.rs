use std::collections::BTreeSet;
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

const TOKEN: &str = "retitle-v3-test-bearer";
const SESSION_ID: &str = "retitle-v3-black-box";
const PROVIDER_SESSION_ID: &str = "provider-retitle-v3-black-box";
const INCARNATION: &str = "launch-retitle-v3-black-box";
const ASSISTANT_CANARY: &str = "assistant-private-output-canary";
const PATH_CANARY: &str = "/private/retitle-v3-canary";
const CREDENTIAL_CANARY: &str = "sk-test-private-retitle-v3-canary";

struct HttpResponse {
    status: u16,
    body: Value,
}

struct ServeProcess {
    child: Child,
    stderr_path: PathBuf,
}

impl ServeProcess {
    fn spawn(fixture: &Fixture) -> Self {
        let stderr_path = fixture
            .root
            .join(format!("serve-{}.stderr", uuid::Uuid::new_v4().simple()));
        let stderr = File::create(&stderr_path).expect("create serve stderr fixture");
        let config = json!({
            "provider": "command",
            "argv": [fixture.provider_bin],
            "timeout_ms": 1000
        })
        .to_string();
        let child = Command::new(bin::resolve("agent-session"))
            .current_dir(&fixture.root)
            .args([
                "serve",
                "--bind",
                &fixture.address.to_string(),
                "--state-dir",
                fixture.state_dir.to_str().expect("UTF-8 state dir"),
                "--token",
                TOKEN,
                "--machine",
                "retitle-v3-test-machine",
                "--tmux-bin",
                fixture.tmux_bin.to_str().expect("UTF-8 tmux fixture"),
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &fixture.home)
            .env("CODEX_HOME", &fixture.codex_home)
            .env("AGENT_SESSION_RETITLE_CONFIG", config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn agent-session serve");
        let mut server = Self { child, stderr_path };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if TcpStream::connect(fixture.address).is_ok() {
                break;
            }
            if let Some(status) = server.child.try_wait().expect("poll serve child") {
                panic!(
                    "agent-session serve exited before listening: status={status}; stderr={}",
                    server.sanitized_stderr()
                );
            }
            assert!(
                Instant::now() < deadline,
                "agent-session serve did not listen; stderr={}",
                server.sanitized_stderr()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        server
    }

    fn stop(&mut self) {
        if self.child.try_wait().expect("poll serve child").is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }

    fn sanitized_stderr(&self) -> String {
        fs::read_to_string(&self.stderr_path)
            .unwrap_or_default()
            .lines()
            .take(8)
            .map(|line| {
                if line.contains('/') {
                    "<path-redacted>"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

impl Drop for ServeProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    state_dir: PathBuf,
    codex_home: PathBuf,
    tmux_bin: PathBuf,
    provider_bin: PathBuf,
    provider_calls: PathBuf,
    record_path: PathBuf,
    address: SocketAddr,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("retitle v3 fixture");
        let root = tmp.path().to_path_buf();
        let home = root.join("home");
        let state_dir = root.join("state");
        let codex_home = root.join("codex-home");
        let provider_calls = root.join("provider.calls");
        let tmux_bin = root.join("tmux");
        let provider_bin = root.join("title-provider");
        fs::create_dir_all(&home).expect("fixture home");
        write_executable(
            &tmux_bin,
            "#!/bin/sh\ncase \"$1\" in\n  has-session) exit 0 ;;\n  *) exit 0 ;;\nesac\n",
        );
        write_executable(
            &provider_bin,
            &format!(
                "#!/bin/sh\nprintf 'called\\n' >> {}\nprintf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"provider must not run\",\"activity\":null,\"references\":[]}}'\n",
                shell_words::quote(&provider_calls.to_string_lossy())
            ),
        );
        let record_path = seed_session(&state_dir);
        seed_codex_transcript(&codex_home);
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback address");
        let address = listener.local_addr().expect("loopback address");
        drop(listener);
        Self {
            _tmp: tmp,
            root,
            home,
            state_dir,
            codex_home,
            tmux_bin,
            provider_bin,
            provider_calls,
            record_path,
            address,
        }
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> HttpResponse {
        request_json(self.address, method, path, token, body)
    }
}

#[test]
fn retitle_v3_routes_are_authenticated_strict_private_and_additive() {
    let fixture = Fixture::new();
    let _server = ServeProcess::spawn(&fixture);
    let opaque_hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    for (method, path, body) in [
        ("GET", "/retitle/v3/readiness", None),
        (
            "GET",
            "/sessions/retitle-v3-black-box/retitle-v3/readiness",
            None,
        ),
        (
            "POST",
            "/sessions/retitle-v3-black-box/retitle-v3",
            Some(manual_request("manual-auth-check", 0, 0)),
        ),
        (
            "GET",
            &format!("/sessions/retitle-v3-black-box/retitle-v3/operations/{opaque_hash}"),
            None,
        ),
    ] {
        let response = fixture.request(method, path, None, body.as_ref());
        assert_eq!(response.status, 401, "path={path} body={}", response.body);
        assert_eq!(response.body["ok"], false, "path={path}");
        assert_eq!(
            response.body["error"]["code"], "unauthorized",
            "path={path}"
        );
    }

    let machine = fixture.request("GET", "/retitle/v3/readiness", Some(TOKEN), None);
    assert_eq!(machine.status, 200, "body={}", machine.body);
    assert_outer_retitle_envelope(&machine.body);
    assert_exact_keys(
        &machine.body["data"]["retitle"],
        &[
            "capability",
            "next_action",
            "reason_code",
            "schema_version",
            "status",
        ],
    );
    assert_eq!(
        machine.body["data"]["retitle"]["schema_version"],
        "agent-session.session-retitle.readiness.v3"
    );
    assert_eq!(
        machine.body["data"]["retitle"]["capability"],
        "agent-session.session-retitle.v3"
    );

    let readiness = fixture.request(
        "GET",
        "/sessions/retitle-v3-black-box/retitle-v3/readiness",
        Some(TOKEN),
        None,
    );
    assert_eq!(readiness.status, 200, "body={}", readiness.body);
    assert_outer_retitle_envelope(&readiness.body);
    assert_exact_keys(
        &readiness.body["data"]["retitle"],
        &[
            "capability",
            "context_status",
            "memory_revision",
            "next_action",
            "pending_operation",
            "provider_status",
            "reason_code",
            "schema_version",
            "status",
            "title_status",
            "usable_memory",
        ],
    );
    assert_eq!(readiness.body["data"]["retitle"]["memory_revision"], 0);

    let mut invalid = manual_request("manual-strict-check", 0, 0);
    invalid["unexpected"] = json!(true);
    let rejected = fixture.request(
        "POST",
        "/sessions/retitle-v3-black-box/retitle-v3",
        Some(TOKEN),
        Some(&invalid),
    );
    assert_eq!(rejected.status, 400, "body={}", rejected.body);
    assert_eq!(rejected.body["ok"], false);

    let admitted = fixture.request(
        "POST",
        "/sessions/retitle-v3-black-box/retitle-v3",
        Some(TOKEN),
        Some(&manual_request("manual-contract-check", 0, 0)),
    );
    assert!(
        matches!(admitted.status, 200 | 202),
        "body={}",
        admitted.body
    );
    assert_outer_retitle_envelope(&admitted.body);
    let operation_hash = admitted.body["data"]["retitle"]["operation_hash"]
        .as_str()
        .expect("operation hash")
        .to_string();
    let terminal = poll_terminal(&fixture, &operation_hash);
    assert_operation_contract(&terminal.body["data"]["retitle"], true);
    assert_eq!(terminal.body["data"]["retitle"]["status"], "terminal");

    let replay = fixture.request(
        "GET",
        &format!("/sessions/retitle-v3-black-box/retitle-v3/operations/{operation_hash}"),
        Some(TOKEN),
        None,
    );
    assert_eq!(replay.status, 200, "body={}", replay.body);
    assert_operation_contract(&replay.body["data"]["retitle"], true);
    assert_eq!(
        replay.body["data"]["retitle"]["operation_hash"],
        operation_hash
    );

    let replay_with_changed_fence = fixture.request(
        "POST",
        "/sessions/retitle-v3-black-box/retitle-v3",
        Some(TOKEN),
        Some(&manual_request("manual-contract-check", 1, 1)),
    );
    assert_eq!(
        replay_with_changed_fence.status, 409,
        "body={}",
        replay_with_changed_fence.body
    );
    assert_eq!(
        replay_with_changed_fence.body["error"]["code"],
        "retitle-v3-idempotency-conflict"
    );

    let sessions = fixture.request("GET", "/sessions", Some(TOKEN), None);
    assert_eq!(sessions.status, 200, "body={}", sessions.body);
    assert_eq!(
        sessions.body["data"]["capabilities"]["session_retitle_v2"],
        true
    );
    assert_eq!(
        sessions.body["data"]["capabilities"]["session_retitle_v3"],
        true
    );
    let v2 = fixture.request("GET", "/retitle/readiness", Some(TOKEN), None);
    assert_eq!(v2.status, 200, "body={}", v2.body);
    assert_eq!(
        v2.body["data"]["retitle"]["schema_version"],
        "agent-session.session-retitle.readiness.v2"
    );

    let persisted: Value =
        serde_json::from_slice(&fs::read(&fixture.record_path).expect("persisted session record"))
            .expect("persisted session record JSON");
    let public_rendered = [
        machine.body.to_string(),
        readiness.body.to_string(),
        admitted.body.to_string(),
        terminal.body.to_string(),
        replay.body.to_string(),
    ]
    .join("\n");
    for (index, forbidden) in [
        TOKEN,
        "manual-contract-check",
        PROVIDER_SESSION_ID,
        ASSISTANT_CANARY,
        PATH_CANARY,
        CREDENTIAL_CANARY,
        fixture.root.to_string_lossy().as_ref(),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            !public_rendered.contains(forbidden),
            "retitle v3 HTTP evidence leaked forbidden fixture value {index}"
        );
    }
    let private_marker = persisted["session_retitle_v3"].to_string();
    for (index, forbidden) in [
        "manual-contract-check",
        PROVIDER_SESSION_ID,
        PATH_CANARY,
        CREDENTIAL_CANARY,
        fixture.root.to_string_lossy().as_ref(),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            !private_marker.contains(forbidden),
            "retitle v3 private marker leaked forbidden fixture value {index}"
        );
    }
}

#[test]
fn retitle_v3_claimed_operation_is_reconciled_after_a_real_daemon_restart() {
    let fixture = Fixture::new();
    let provider_started = fixture.root.join("provider.started");
    write_executable(
        &fixture.provider_bin,
        &format!(
            "#!/bin/sh\nprintf 'called\\n' >> {}\n: > {}\nparent=$PPID\nwhile kill -0 \"$parent\" 2>/dev/null; do sleep 0.01; done\nexit 143\n",
            shell_words::quote(&fixture.provider_calls.to_string_lossy()),
            shell_words::quote(&provider_started.to_string_lossy()),
        ),
    );
    let mut record: Value = serde_json::from_slice(
        &fs::read(&fixture.record_path).expect("read session record before admission"),
    )
    .expect("session record JSON");
    record["title"] = json!("Existing restart-safe title");
    record["title_revision"] = json!(1);
    write_private_json(&fixture.record_path, &record);
    seed_automatic_activity(&fixture.state_dir, "restart-turn", 1);

    let mut first_daemon = ServeProcess::spawn(&fixture);
    let admitted = fixture.request(
        "POST",
        "/sessions/retitle-v3-black-box/retitle-v3",
        Some(TOKEN),
        Some(&automatic_request(
            "automatic-restart",
            1,
            0,
            1,
            "restart-turn",
        )),
    );
    assert_eq!(admitted.status, 202, "body={}", admitted.body);
    let operation_hash = admitted.body["data"]["retitle"]["operation_hash"]
        .as_str()
        .expect("admitted operation hash")
        .to_string();
    wait_for_path(&provider_started);
    first_daemon.stop();

    let mut persisted: Value = serde_json::from_slice(
        &fs::read(&fixture.record_path).expect("read claimed operation after daemon stop"),
    )
    .expect("claimed operation JSON");
    let receipt = persisted["session_retitle_v3"]["receipts"]
        .as_array_mut()
        .expect("v3 receipts")
        .iter_mut()
        .find(|receipt| receipt["operation_hash"] == operation_hash)
        .expect("claimed operation receipt");
    assert!(receipt["execution_claim"].is_object());
    // Advance the durable lease boundary without sleeping 150 seconds. The
    // second daemon must reconcile the uncertain attempt, not invoke it again.
    receipt["execution_claim"]["expires_at_second"] = json!(0);
    write_private_json(&fixture.record_path, &persisted);

    let _second_daemon = ServeProcess::spawn(&fixture);
    let first = fixture.request(
        "GET",
        &format!("/sessions/retitle-v3-black-box/retitle-v3/operations/{operation_hash}"),
        Some(TOKEN),
        None,
    );
    assert_eq!(first.status, 200, "body={}", first.body);
    let terminal = if first.body["data"]["retitle"]["status"] == "terminal" {
        first
    } else {
        poll_terminal(&fixture, &operation_hash)
    };
    assert_eq!(terminal.body["data"]["retitle"]["status"], "terminal");
    assert_eq!(
        terminal.body["data"]["retitle"]["operation_hash"],
        operation_hash
    );
    assert_eq!(
        terminal.body["data"]["retitle"]["outcome"],
        "degraded_cached"
    );
    assert_eq!(
        terminal.body["data"]["retitle"]["failure_class"],
        "uncertain_execution"
    );
    assert_eq!(
        terminal.body["data"]["retitle"]["title"],
        "Existing restart-safe title"
    );
    assert_operation_contract(&terminal.body["data"]["retitle"], true);
    let stored: Value = serde_json::from_slice(
        &fs::read(&fixture.record_path).expect("read adopted operation state"),
    )
    .expect("adopted session record JSON");
    let receipt = stored["session_retitle_v3"]["receipts"]
        .as_array()
        .expect("v3 receipts")
        .iter()
        .find(|receipt| receipt["operation_hash"] == operation_hash)
        .expect("adopted receipt");
    assert_eq!(receipt["state"], "degraded_cached");
    assert_eq!(receipt["execution_claim"], Value::Null);
    assert_eq!(
        fs::read_to_string(&fixture.provider_calls)
            .expect("one provider call before restart")
            .lines()
            .count(),
        1
    );
}

#[test]
fn retitle_v3_fresh_manual_cache_avoids_provider_calls_and_reports_transport_latency() {
    let fixture = Fixture::new();
    let _server = ServeProcess::spawn(&fixture);
    let first = fixture.request(
        "POST",
        "/sessions/retitle-v3-black-box/retitle-v3",
        Some(TOKEN),
        Some(&manual_request("manual-prime-cache", 0, 0)),
    );
    assert!(matches!(first.status, 200 | 202), "body={}", first.body);
    let operation_hash = first.body["data"]["retitle"]["operation_hash"]
        .as_str()
        .expect("prime operation hash");
    let primed = poll_terminal(&fixture, operation_hash);
    let mut title_revision = primed.body["data"]["retitle"]["title_revision"]
        .as_u64()
        .expect("title revision");
    let mut memory_revision = primed.body["data"]["retitle"]["memory_revision"]
        .as_u64()
        .expect("memory revision");
    let mut elapsed = Vec::new();
    for index in 0..21 {
        let request = manual_request(
            &format!("manual-cached-{index:04}"),
            title_revision,
            memory_revision,
        );
        let started = Instant::now();
        let response = fixture.request(
            "POST",
            "/sessions/retitle-v3-black-box/retitle-v3",
            Some(TOKEN),
            Some(&request),
        );
        elapsed.push(started.elapsed());
        assert_eq!(response.status, 200, "index={index} body={}", response.body);
        assert_eq!(response.body["data"]["retitle"]["status"], "terminal");
        assert!(matches!(
            response.body["data"]["retitle"]["outcome"].as_str(),
            Some("committed" | "unchanged")
        ));
        assert_operation_contract(&response.body["data"]["retitle"], true);
        title_revision = response.body["data"]["retitle"]["title_revision"]
            .as_u64()
            .expect("current title revision");
        memory_revision = response.body["data"]["retitle"]["memory_revision"]
            .as_u64()
            .expect("current memory revision");
    }
    elapsed.sort_unstable();
    let p95_index = ((elapsed.len() * 95).div_ceil(100)).saturating_sub(1);
    let p95 = elapsed[p95_index];
    eprintln!(
        "retitle-v3 fresh-cache samples={} p95_ms={} provider_requests=0",
        elapsed.len(),
        p95.as_millis()
    );
    // The loopback request includes process scheduling and TCP setup/teardown,
    // so its wall clock is diagnostic rather than a deterministic correctness
    // gate. The in-process latency regression owns the 250 ms service budget;
    // this black-box lane proves that the provider boundary stays unused.
    assert!(
        !fixture.provider_calls.exists(),
        "fresh manual operations invoked the configured provider"
    );
}

fn poll_terminal(fixture: &Fixture, operation_hash: &str) -> HttpResponse {
    let path = format!("/sessions/retitle-v3-black-box/retitle-v3/operations/{operation_hash}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let response = fixture.request("GET", &path, Some(TOKEN), None);
        assert_eq!(response.status, 200, "body={}", response.body);
        if response.body["data"]["retitle"]["status"] == "terminal" {
            return response;
        }
        assert_eq!(response.body["data"]["retitle"]["status"], "accepted");
        assert!(
            Instant::now() < deadline,
            "retitle v3 operation did not become terminal: body={}",
            response.body
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn manual_request(key: &str, title_revision: u64, memory_revision: u64) -> Value {
    json!({
        "schema_version": "agent-session.session-retitle.request.v3",
        "trigger": "manual",
        "idempotency_key": key,
        "expected": {
            "session_incarnation": INCARNATION,
            "title_revision": title_revision,
            "memory_revision": memory_revision
        }
    })
}

fn automatic_request(
    key: &str,
    title_revision: u64,
    memory_revision: u64,
    activity_revision: u64,
    provider_turn_id: &str,
) -> Value {
    json!({
        "schema_version": "agent-session.session-retitle.request.v3",
        "trigger": "automatic",
        "idempotency_key": key,
        "expected": {
            "session_incarnation": INCARNATION,
            "title_revision": title_revision,
            "memory_revision": memory_revision,
            "activity_revision": activity_revision,
            "provider_turn_id": provider_turn_id
        }
    })
}

fn seed_automatic_activity(state_dir: &Path, provider_turn_id: &str, revision: u64) {
    let activity_path = state_dir
        .join("sessions")
        .join(SESSION_ID)
        .join("activity.json");
    write_private_json(
        &activity_path,
        &json!({
            "schema_version": "agent-session.activity.v1",
            "runtime_id": INCARNATION,
            "runtime_generation": 1,
            "state": {
                "schema_version": "agent-session.turn-state.v1",
                "phase": "working",
                "phase_changed_at": "2026-09-09T00:00:03Z",
                "revision": revision,
                "source": {
                    "kind": "provider_hook",
                    "provider": "codex",
                    "confidence": "authoritative"
                },
                "current_turn": {
                    "provider_turn_id": provider_turn_id,
                    "started_at": "2026-09-09T00:00:03Z"
                }
            }
        }),
    );
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "provider did not reach deterministic restart barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_outer_retitle_envelope(body: &Value) {
    assert_exact_keys(body, &["data", "ok", "schema_version"]);
    assert_eq!(body["schema_version"], "cli.agent-session.serve.v1");
    assert_eq!(body["ok"], true);
    assert_exact_keys(&body["data"], &["machine", "retitle"]);
    assert_eq!(body["data"]["machine"], "retitle-v3-test-machine");
}

fn assert_operation_contract(operation: &Value, terminal: bool) {
    let mut keys = vec![
        "admission_fence",
        "capability",
        "current_fence",
        "duration_bucket",
        "memory_revision",
        "operation_hash",
        "readiness",
        "result_is_current",
        "schema_version",
        "session_incarnation",
        "started_at",
        "status",
        "title_revision",
    ];
    if operation.get("title").is_some() {
        keys.push("title");
    }
    if terminal {
        keys.extend(["changed", "finished_at", "outcome", "result_fence"]);
    }
    if operation.get("failure_class").is_some() {
        keys.push("failure_class");
    }
    if operation.get("failure_stage").is_some() {
        keys.push("failure_stage");
    }
    if operation.get("provider_attempts").is_some() {
        keys.push("provider_attempts");
    }
    assert_exact_keys(operation, &keys);
    assert_eq!(
        operation["schema_version"],
        "agent-session.session-retitle.v3"
    );
    assert_eq!(operation["capability"], "agent-session.session-retitle.v3");
    assert_eq!(
        operation["status"],
        if terminal { "terminal" } else { "accepted" }
    );
    assert_eq!(operation.get("outcome").is_some(), terminal);
    assert_eq!(operation.get("changed").is_some(), terminal);
    assert_eq!(operation.get("finished_at").is_some(), terminal);
    assert_eq!(operation.get("result_fence").is_some(), terminal);
    if !terminal {
        assert_eq!(operation["result_is_current"], false);
    }
    assert_exact_keys(
        &operation["admission_fence"],
        &["memory_revision", "session_incarnation", "title_revision"],
    );
    assert_exact_keys(
        &operation["current_fence"],
        &["memory_revision", "session_incarnation", "title_revision"],
    );
    if terminal {
        assert_exact_keys(
            &operation["result_fence"],
            &["memory_revision", "session_incarnation", "title_revision"],
        );
    }
}

fn assert_exact_keys(value: &Value, expected: &[&str]) {
    let actual = value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

fn request_json(
    address: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> HttpResponse {
    let mut stream = TcpStream::connect(address).expect("connect agent-session serve");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("HTTP read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .expect("HTTP write timeout");
    let body = body.map(Value::to_string).unwrap_or_default();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\n").expect("HTTP request line");
    if let Some(token) = token {
        write!(stream, "Authorization: Bearer {token}\r\n").expect("HTTP authorization");
    }
    if !body.is_empty() {
        write!(
            stream,
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )
        .expect("HTTP content headers");
    }
    write!(stream, "Connection: close\r\n\r\n{body}").expect("HTTP body");
    stream.flush().expect("flush HTTP request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read HTTP response");
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response header boundary");
    let headers = std::str::from_utf8(&response[..header_end]).expect("HTTP response headers");
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .expect("HTTP response status");
    let body = serde_json::from_slice(&response[header_end + 4..]).expect("HTTP JSON response");
    HttpResponse { status, body }
}

fn seed_session(state_dir: &Path) -> PathBuf {
    let directory = state_dir.join("sessions").join(SESSION_ID);
    fs::create_dir_all(&directory).expect("managed session directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .expect("managed session directory mode");
    let record_path = directory.join("session.json");
    write_private_json(
        &record_path,
        &json!({
            "schema_version": "agent-session.session.v1",
            "id": SESSION_ID,
            "agent": "codex",
            "mode": "interactive",
            "title": null,
            "title_revision": 0,
            "cwd": "/synthetic/retitle-v3-worktree",
            "tmux_session": "hs-retitle-v3-black-box",
            "prompt_file": null,
            "log_file": null,
            "created_at": "2026-09-09T00:00:00Z",
            "updated_at": "2026-09-09T00:00:00Z",
            "provider_resume": {
                "provider": "codex",
                "session_id": PROVIDER_SESSION_ID,
                "captured_at": "2026-09-09T00:00:03Z",
                "capture_method": "synthetic_fixture",
                "resume_args": []
            },
            "runtime": {
                "kind": "tmux",
                "tmux_session": "hs-retitle-v3-black-box",
                "generation": 1,
                "started_at": "2026-09-09T00:00:00Z",
                "launch_id": INCARNATION
            }
        }),
    );
    record_path
}

fn seed_codex_transcript(codex_home: &Path) {
    let directory = codex_home.join("sessions/2026/09/09");
    fs::create_dir_all(&directory).expect("Codex history directory");
    let transcript = directory.join("rollout-retitle-v3-black-box.jsonl");
    let records = [
        json!({
            "timestamp": "2026-09-09T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": PROVIDER_SESSION_ID,
                "cwd": "/synthetic/retitle-v3-worktree",
                "source": "cli",
                "timestamp": "2026-09-09T00:00:00Z"
            }
        }),
        json!({
            "timestamp": "2026-09-09T00:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": "Make long session retitle reliable with bounded semantic memory"
                }],
                "internal_chat_message_metadata_passthrough": {
                    "turn_id": "synthetic-human-turn",
                    "content_item_kinds": ["user.text"]
                }
            }
        }),
        json!({
            "timestamp": "2026-09-09T00:00:02Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": format!(
                        "{ASSISTANT_CANARY} {PATH_CANARY} {CREDENTIAL_CANARY}"
                    )
                }]
            }
        }),
    ];
    let rendered = records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&transcript, rendered).expect("Codex history fixture");
    fs::set_permissions(&transcript, fs::Permissions::from_mode(0o600))
        .expect("Codex history fixture mode");
}

fn write_private_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("fixture JSON"),
    )
    .expect("write private JSON fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .expect("private JSON fixture mode");
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable fixture mode");
}
