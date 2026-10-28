//! Real CLI HTTP serialization and target launch, with disposable provider fixtures.
use super::*;
use nils_test_support::cmd::CmdOutput;
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const TOKEN: &str = "synthetic-runtime-capability-000000000000001";

fn private(path: &Path, value: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(path, value).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn seed_caller(f: &Fixture, url: &str) -> PathBuf {
    let digest = |value: &str| {
        Sha256::digest(value.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let capability = f.state.join(format!(
        "sessions/source/coordination/capability-{}",
        digest("source-runtime")
    ));
    private(&capability, TOKEN.as_bytes());
    fs::set_permissions(
        f.state.join("sessions/source"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    private(
        &f.state.join("sessions/source/coordination/heartbeat"),
        format!("source-runtime:{now}\n").as_bytes(),
    );
    private(&f.state.join("coordination/registry.json"),json!({"schema_version":"agent-session.coordination-registry.v1","brokers":{
        "source":{"session_id":"source","incarnation":"source-runtime","capability_digest":digest(TOKEN),"generation":1,
            "state":"ready","heartbeat_at":"2026-01-01T00:00:00Z","heartbeat_epoch":now}}}).to_string().as_bytes());
    private(
        &f.state.join("coordination/daemon-endpoint.json"),
        json!({"url":url}).to_string().as_bytes(),
    );
    capability
}

fn run_console(f: &Fixture, capability: &Path, extra: &[&str]) -> CmdOutput {
    let options = CmdOptions::new()
        .with_env_remove_prefix("AGENT_SESSION_")
        .with_env("HOME", f.tmp.path().to_str().unwrap())
        .with_env(
            "AGENT_HOME",
            f.tmp.path().join("agent-home").to_str().unwrap(),
        )
        .with_env("AGENT_SESSION_ID", "source")
        .with_env(
            "AGENT_SESSION_CAPABILITY_FILE",
            capability.to_str().unwrap(),
        );
    let mut args = vec![
        "--state-dir",
        f.state.to_str().unwrap(),
        "start",
        "--agent",
        "claude",
        "--via-console",
        "--machine",
        "target",
        "--resume",
        CONVERSATION,
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    run_resolved("agent-session", &args, &options)
}

fn preflight() -> Value {
    json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,"data":{
        "agent":"claude","provider_session_id":CONVERSATION,"machine":"target","account":"original"}})
}

fn read_request(stream: &mut TcpStream) -> (String, Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let (header_end, length) = loop {
        let mut chunk = [0; 4096];
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            assert!(headers.contains(&format!("authorization: bearer {TOKEN}")));
            let length = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
    }
    let path = String::from_utf8_lossy(&bytes[..header_end])
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    (
        path,
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
    )
}

fn stub(responses: Vec<(u16, Value)>) -> (String, thread::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(Instant::now() < deadline, "expected CLI HTTP request");
                thread::sleep(Duration::from_millis(10));
            };
            requests.push(read_request(&mut stream));
            let body = if status == 404 && body.is_null() {
                String::new()
            } else {
                body.to_string()
            };
            write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        requests
    });
    (format!("http://{address}"), worker)
}

#[test]
fn cli_console_check_does_not_read_prompt_or_create_operation_and_rejects_old_or_misbound_answers()
{
    for (answer, expected) in [
        (preflight(), None),
        (
            json!({"schema_version":"agent-session.serve.v1","ok":true}),
            Some("resume-unsupported"),
        ),
        {
            let mut answer = preflight();
            answer["data"]["machine"] = json!("other-target");
            (answer, Some("resume-unsupported"))
        },
        {
            let mut answer = preflight();
            answer["data"]["account"] = json!("/private/fixture-account");
            (answer, Some("resume-unsupported"))
        },
        {
            let mut answer = preflight();
            answer["data"]["history_scope"] = json!("account");
            (answer, None)
        },
        {
            let mut answer = preflight();
            answer["data"]["history_scope"] = json!("shared");
            (answer, None)
        },
        {
            let mut answer = preflight();
            answer["data"]["history_scope"] = json!("unexpected");
            (answer, Some("resume-unsupported"))
        },
    ] {
        let f = Fixture::new();
        let expected_success = answer.clone();
        let (url, worker) = stub(vec![(200, answer)]);
        let capability = seed_caller(&f, &url);
        let source = fs::read(f.record_path("source")).unwrap();
        let registry = fs::read(f.state.join("coordination/registry.json")).unwrap();
        let out = run_console(
            &f,
            &capability,
            &["--check-resume", "--prompt-file", "/nonexistent/prompt"],
        );
        let requests = worker.join().unwrap();
        assert_eq!(requests[0].0, "/sessions/source/resume-preflight/v1");
        assert_eq!(
            requests[0].1,
            json!({"machine":"target","session":{"agent":"claude","resume":CONVERSATION}})
        );
        match expected {
            None => assert_eq!(out.stdout_json(), expected_success, "{}", out.stderr_text()),
            Some(code) => {
                assert_eq!(out.stdout_json()["error"]["code"], code);
                assert_eq!(
                    out.stdout_json()["error"]["details"]["session_created"],
                    false
                );
            }
        }
        assert_eq!(fs::read(f.record_path("source")).unwrap(), source);
        assert_eq!(
            fs::read(f.state.join("coordination/registry.json")).unwrap(),
            registry
        );
        assert_eq!(
            fs::read_dir(f.state.join("coordination")).unwrap().count(),
            2
        );
    }
}

#[test]
fn old_daemon_without_a_preflight_route_is_distinguishable_from_transport_failure() {
    let f = Fixture::new();
    let (url, worker) = stub(vec![(404, Value::Null), (404, Value::Null)]);
    let capability = seed_caller(&f, &url);
    let out = run_console(&f, &capability, &["--check-resume"]);
    let start = run_console(&f, &capability, &[]);
    assert_eq!(start.stdout_json()["error"]["code"], "resume-unsupported");
    assert_eq!(
        start.stdout_json()["error"]["details"]["session_created"],
        false
    );
    worker.join().unwrap();
    assert_eq!(out.stdout_json()["error"]["code"], "resume-unsupported");
    assert_eq!(
        out.stdout_json()["error"]["details"]["safe_to_retry"],
        false
    );
}

#[test]
fn cli_console_start_forwards_resume_account_and_ordinary_context_and_terminal_refusals() {
    for code in [
        None,
        Some("resume-source-live"),
        Some("resume-preflight-unavailable"),
        Some("resume-account-transition-pending"),
    ] {
        let f = Fixture::new();
        let result = if let Some(code) = code {
            json!({"error":{"code":code,"message":"fixture refusal","details":{"session_created":false,"safe_to_retry":false}}})
        } else {
            json!({"schema_version":"agent-session.console-start.v1","machine":"target","session":{"id":"new-child","title_mode":"pinned","display_revision":1}})
        };
        let (url, worker) = stub(vec![(if code.is_some() { 422 } else { 201 }, result)]);
        let capability = seed_caller(&f, &url);
        let out = run_console(
            &f,
            &capability,
            &[
                "--account",
                "original",
                "--no-parent",
                "--role",
                "coordinator",
                "--workspace",
                "example",
                "--issue",
                "example/project#7",
                "--title",
                "Resume fixture",
                "--title-mode",
                "pinned",
                "--agent-arg=--model",
                "--agent-arg=fixture-model",
                "--prompt",
                "fixture bootstrap",
            ],
        );
        let requests = worker.join().unwrap();
        assert_eq!(requests[0].0, "/sessions/source/console-resume/v1");
        let request = &requests[0].1;
        assert_eq!(request["session"]["resume"], CONVERSATION);
        assert_eq!(request["session"]["claude_account"], "original");
        assert_eq!(request["session"]["prompt"], "fixture bootstrap");
        assert_eq!(
            request["session"]["agent_args"],
            json!(["--model", "fixture-model"])
        );
        assert_eq!(request["role"], "coordinator");
        assert!(
            request["session"].get("cwd").is_none(),
            "omitted cwd resolves on the target"
        );
        assert_eq!(
            out.stdout_json()["schema_version"],
            "cli.agent-session.start.v1"
        );
        if let Some(code) = code {
            assert_eq!(out.stdout_json()["error"]["code"], code);
            assert_eq!(
                out.stdout_json()["error"]["details"]["safe_to_retry"],
                false
            );
        } else {
            assert_eq!(out.code, 0, "{}", out.stderr_text());
            assert_eq!(out.stdout_json()["data"]["session"]["id"], "new-child");
        }
    }
}

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn target_http_resume_creates_new_identity_and_preserves_account_context_and_prompt() {
    let f = Fixture::new();
    super::super::cli::fake_tmux(f.tmp.path());
    super::super::cli::fake_agent(f.tmp.path(), "provider");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let child = Command::new(nils_test_support::bin::resolve("agent-session"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", f.tmp.path())
        .env("AGENT_HOME", f.tmp.path().join("agent-home"))
        .env("CLAUDE_CONFIG_DIR", f.tmp.path().join("unrelated-default"))
        .env(
            "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
            json!([f.broker]).to_string(),
        )
        .env("AGENT_SESSION_CLAUDE_BIN", f.tmp.path().join("provider"))
        .env("AGENT_SESSION_FAKE_TMUX_LOG", f.tmp.path().join("tmux.log"))
        .env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1")
        .arg("--state-dir")
        .arg(&f.state)
        .args([
            "serve",
            "--bind",
            &address.to_string(),
            "--machine",
            "target",
            "--token",
            TOKEN,
            "--tmux-bin",
        ])
        .arg(&f.tmux)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut daemon = Daemon(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(address).is_err() {
        assert!(daemon.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let before = fs::read(f.record_path("source")).unwrap();
    let response=reqwest::blocking::Client::new().post(format!("http://{address}/sessions/resume/v1")).bearer_auth(TOKEN)
        .json(&json!({"agent":"claude","resume":CONVERSATION,"id":"http-child","role":"coordinator","workspace":"example",
            "title":"Resume fixture","title_mode":"pinned","agent_args":["--model","fixture-model"],"prompt":"fixture bootstrap",
            "origin":{"parent":{"machine":"origin","session_id":"caller"}},
            "work":{"issues":[{"provider":"github","repository":"example/project","number":7}],"inherited":false}})).send().unwrap();
    let status = response.status();
    let result: Value = response.json().unwrap();
    assert!(status.is_success(), "{result}");
    let created = &result["data"]["session"];
    assert_eq!(created["id"], "http-child");
    assert_eq!(created["provider_resume"]["session_id"], CONVERSATION);
    assert_eq!(created["claude_account"]["selected_account"], "original");
    assert_eq!(created["role"], "coordinator");
    assert_eq!(created["workspace"], "example");
    assert_eq!(created["title_mode"], "pinned");
    assert_eq!(
        fs::read_to_string(f.state.join("sessions/http-child/prompt.md")).unwrap(),
        "fixture bootstrap"
    );
    assert_eq!(fs::read(f.record_path("source")).unwrap(), before);
}

// This origin-daemon fixture retains an operation result. A later admission
// request is deliberately refused; a replay must reach the receipt instead.
fn operation_stub(outage: bool) -> (String, thread::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        let mut created = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => break,
            };
            let (path, request) = read_request(&mut stream);
            let stop = path == "/stop";
            let (status, body) = if stop {
                (200, json!({}))
            } else if path.ends_with("/resume-preflight/v1") {
                if created {
                    (
                        if outage { 503 } else { 422 },
                        json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":false,
                        "error":{"code":if outage {"resume-preflight-unavailable"} else {"resume-source-live"},
                            "details":{"session_created":false,"safe_to_retry":false}}}),
                    )
                } else {
                    (200, preflight())
                }
            } else {
                let mut result = json!({"schema_version":"agent-session.console-start.v1","machine":"target","session":{"id":"new-child"}});
                if created {
                    result["reconciliation"] = json!({"state":"created","safe_to_retry":false});
                }
                created = true;
                (201, result)
            };
            let body = body.to_string();
            write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            if stop {
                break;
            }
            requests.push((path, request));
        }
        requests
    });
    (format!("http://{address}"), worker)
}

fn assert_console_replay(outage: bool) {
    let f = Fixture::new();
    let (url, worker) = operation_stub(outage);
    let capability = seed_caller(&f, &url);
    let operation = uuid::Uuid::now_v7().to_string();
    let args = [
        "--operation-id",
        operation.as_str(),
        "--prompt",
        "fixture bootstrap",
    ];
    let first = run_console(&f, &capability, &args);
    let second = run_console(&f, &capability, &args);
    reqwest::blocking::Client::new()
        .post(format!("{url}/stop"))
        .bearer_auth(TOKEN)
        .json(&json!({}))
        .send()
        .unwrap();
    let requests = worker.join().unwrap();
    assert_eq!(first.code, 0, "{}", first.stdout_text());
    assert_eq!(
        second.code,
        0,
        "replay must return the known child: {}",
        second.stdout_text()
    );
    assert_eq!(first.stdout_json()["data"]["session"]["id"], "new-child");
    assert_eq!(second.stdout_json()["data"]["session"]["id"], "new-child");
    assert_eq!(
        requests.len(),
        2,
        "CLI replay must not repeat provider admission"
    );
    for (path, request) in requests {
        assert_eq!(path, "/sessions/source/console-resume/v1");
        assert_eq!(request["operation_id"], operation);
        assert_eq!(request["session"]["resume"], CONVERSATION);
    }
}

#[test]
fn console_replay_returns_created_child_when_the_conversation_is_now_live() {
    assert_console_replay(false);
}

#[test]
fn console_replay_returns_created_child_during_a_later_preflight_outage() {
    assert_console_replay(true);
}
