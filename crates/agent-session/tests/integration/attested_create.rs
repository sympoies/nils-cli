//! Destination-side Console attestation through the loopback serve API.
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nils_test_support::tempdir::ScopedTempDir;
use pretty_assertions::assert_eq;
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{cli, serve_fixture};

const MACHINE: &str = "fixture-destination";
const BEARER: &str = "fixture-machine-bearer";
const TOKEN: &str = "fixture-attested-create";
const MAILBOX: &str = "synthetic-fixture-mailbox-ingress-token";
const TOKEN_HEADER: &str = "x-agent-session-attested-create-token";
const BINDING_HEADER: &str = "x-agent-session-console-create-binding";
const MEDIA: &str = "application/vnd.agent-session.console-attested-create.v1";

struct Daemon {
    child: Child,
    root: ScopedTempDir,
    client: Client,
    url: String,
}

impl Daemon {
    fn spawn(bearer: Option<&str>, attested_token: Option<&str>) -> Self {
        let root = ScopedTempDir::with_prefix("attested-create-");
        let state = root.path().join("state");
        let (tmux, log) = cli::fake_tmux(root.path());
        let provider = root.path().join("provider");
        cli::write_executable(
            &provider,
            "#!/bin/sh\ntest -z \"${AGENT_SESSION_CONSOLE_ATTESTED_CREATE_TOKEN+x}\"\n",
        );
        let config = root.path().join("provider-config");
        fs::create_dir(&config).unwrap();
        let profiles = json!([{
            "id": "fixture", "label": "Fixture", "agent": "claude",
            "agent_bin": provider, "provider_config_dir": config,
            "readiness_args": ["--check"]
        }]);
        let mut command = serve_fixture::command(
            root.path(),
            nils_test_support::bin::resolve("agent-session"),
            &state,
            &tmux,
            |command| {
                command
                    .args(["--bind", "127.0.0.1:0", "--machine", MACHINE])
                    .env("AGENT_SESSION_FAKE_TMUX_LOG", log)
                    .env("AGENT_SESSION_FAKE_TMUX_LIST_WINDOWS", "")
                    .env("AGENT_SESSION_LAUNCH_PROFILES", profiles.to_string())
                    .env("AGENT_SESSION_RELAY_INGRESS_TOKEN", MAILBOX)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null());
            },
        );
        if let Some(bearer) = bearer {
            command.env("AGENT_SESSION_TOKEN", bearer);
        }
        if let Some(token) = attested_token {
            command.env("AGENT_SESSION_CONSOLE_ATTESTED_CREATE_TOKEN", token);
        }
        if bearer.is_some() {
            command
                .env("AGENT_SESSION_RELAY_URL", "http://127.0.0.1:1")
                .env(
                    "AGENT_SESSION_RELAY_TOKEN",
                    "synthetic-fixture-mailbox-outbound-token",
                );
        } else {
            command.env_remove("AGENT_SESSION_RELAY_INGRESS_TOKEN");
        }
        let mut daemon = Self {
            child: command.process_group(0).spawn().unwrap(),
            root,
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap(),
            url: String::new(),
        };
        let endpoint = state.join("coordination/daemon-endpoint.json");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(raw) = fs::read(&endpoint) {
                let endpoint: Value = serde_json::from_slice(&raw).unwrap();
                daemon.url = endpoint["url"].as_str().unwrap().to_owned();
                return daemon;
            }
            assert_eq!(daemon.child.try_wait().unwrap(), None);
            assert!(
                Instant::now() < deadline,
                "fixture daemon endpoint unavailable"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn body(&self, extra: Value) -> Vec<u8> {
        let mut body = json!({
            "agent": "claude", "agent_profile": "fixture",
            "id": "fixture-steward", "cwd": self.root.path()
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::to_vec(&body).unwrap()
    }

    fn post(&self, headers: HeaderMap, body: &[u8]) -> (u16, String) {
        let response = self
            .client
            .post(format!("{}/sessions", self.url))
            .headers(headers)
            .body(body.to_vec())
            .send()
            .unwrap();
        (response.status().as_u16(), response.text().unwrap())
    }

    fn record(&self, id: &str) -> Value {
        let path = self
            .root
            .path()
            .join("state/sessions")
            .join(id)
            .join("session.json");
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn broker_identity(&self, incarnation: &str) -> Value {
        let state_dir = self.root.path().join("state");
        let context = agent_session::CliContext {
            state_dir: state_dir.clone(),
            host: None,
        };
        let capability =
            agent_session::coordination::capability_path(&context, "fixture-steward", incarnation);
        let output = Command::new(nils_test_support::bin::resolve("agent-session"))
            .env_clear()
            .env("PATH", serve_fixture::SYSTEM_PATH)
            .env("HOME", self.root.path().join("home"))
            .env("AGENT_SESSION_STATE_DIR", state_dir)
            .env("AGENT_SESSION_ID", "fixture-steward")
            .env("AGENT_SESSION_RUNTIME_ID", incarnation)
            .args([
                "broker",
                "identity",
                "--session",
                "fixture-steward",
                "--format",
                "json",
                "--capability-file",
            ])
            .arg(capability)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "broker identity: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
    }

    fn assert_counts(&self, records: usize, launches: usize) {
        let state = self.root.path().join("state/sessions");
        let count = fs::read_dir(state)
            .into_iter()
            .flatten()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("session.json")
                    .is_file()
            })
            .count();
        assert_eq!(count, records);
        let calls = cli::tmux_calls(&self.root.path().join("tmux.log"));
        assert_eq!(
            calls
                .iter()
                .filter(|args| args.first().is_some_and(|arg| arg == "new-session"))
                .count(),
            launches
        );
        assert!(
            calls.iter().flatten().all(|arg| !arg.contains(TOKEN)),
            "credential reached tmux"
        );
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let pid = self.child.id() as libc::pid_t;
        // Let serve drain blocking tmux probes, which run in their own groups.
        // SAFETY: pid identifies the daemon spawned by this fixture.
        unsafe { libc::kill(pid, libc::SIGINT) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            // SAFETY: waitid writes the status of this fixture's child into a valid buffer.
            // WNOWAIT reserves its PID until we have signaled the whole group.
            let exited = unsafe {
                let mut status: libc::siginfo_t = std::mem::zeroed();
                let result = libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut status,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                );
                if result < 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)
                {
                    return;
                }
                result == 0 && status.si_pid() == pid
            };
            if exited {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }

        let group = -pid;
        // SAFETY: spawn made the daemon the leader of its own process group.
        unsafe { libc::kill(group, libc::SIGKILL) };
        let _ = self.child.wait();
        let deadline = Instant::now() + Duration::from_secs(5);
        // SAFETY: signal 0 checks whether a process remains in that group.
        while unsafe { libc::kill(group, 0) } == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn binding(body: &[u8], role: &str) -> String {
    let binding = json!({
        "schema_version": "agent-session.console-create-binding.v1",
        "scope": "create",
        "origin_machine": "fixture-origin",
        "caller_session_id": "fixture-dispatcher",
        "caller_session_incarnation": "5aa62c37-d456-4c23-9122-95b5281757e1",
        "destination_machine": MACHINE,
        "attested_role": role,
        "body_sha256": Sha256::digest(body).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
    });
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&binding).unwrap())
}

fn headers(body: &[u8], role: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(MEDIA));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer fixture-machine-bearer"),
    );
    headers.insert(TOKEN_HEADER, HeaderValue::from_static(TOKEN));
    headers.insert(
        BINDING_HEADER,
        HeaderValue::from_str(&binding(body, role)).unwrap(),
    );
    headers
}

fn refusal((status, body): (u16, String), expected: u16, code: &str) {
    assert_eq!(status, expected, "{body}");
    let response: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], code);
    assert!(!body.contains(TOKEN));
    assert!(!body.contains(MAILBOX));
    if code == "console-create-invalid-body" {
        assert_eq!(
            response["error"]["message"],
            "invalid attested create request"
        );
    }
}

#[test]
fn attested_create_requires_both_credentials_before_checking_binding() {
    let daemon = Daemon::spawn(Some(BEARER), Some(TOKEN));
    let body = daemon.body(json!({}));
    for name in [TOKEN_HEADER, AUTHORIZATION.as_str()] {
        let mut request = headers(&body, "steward");
        request.remove(name);
        refusal(daemon.post(request, &body), 401, "unauthorized");
        let mut request = headers(&body, "steward");
        request.append(name, request[name].clone());
        refusal(daemon.post(request, &body), 401, "unauthorized");
    }
    let mut request = headers(&body, "steward");
    request.insert(TOKEN_HEADER, HeaderValue::from_static(MAILBOX));
    refusal(daemon.post(request, &body), 401, "unauthorized");
    let mut request = headers(&body, "steward");
    request.remove(BINDING_HEADER);
    refusal(
        daemon.post(request, &body),
        403,
        "console-create-binding-mismatch",
    );
    let mut changed = body.clone();
    changed.push(b' ');
    refusal(
        daemon.post(headers(&body, "steward"), &changed),
        403,
        "console-create-binding-mismatch",
    );
    let mut request = headers(&body, "steward");
    request.append(BINDING_HEADER, request[BINDING_HEADER].clone());
    refusal(
        daemon.post(request, &body),
        403,
        "console-create-binding-mismatch",
    );
    let mut request = headers(&body, "steward");
    let members = [
        "agent-session.console-create-binding.v1",
        "create",
        "fixture-origin",
        "fixture-dispatcher",
        "5aa62c37-d456-4c23-9122-95b5281757e1",
        MACHINE,
        "steward",
        &agent_session::coordination::digest_bytes(&body),
    ];
    let positional = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&members).unwrap());
    request.insert(BINDING_HEADER, HeaderValue::from_str(&positional).unwrap());
    refusal(
        daemon.post(request, &body),
        403,
        "console-create-binding-mismatch",
    );
    let mut request = headers(&body, "steward");
    request.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    assert_eq!(daemon.post(request, &body).0, 415);
    daemon.assert_counts(0, 0);
}

#[test]
fn attested_create_discloses_unavailable_configuration_only_to_machine_bearer() {
    for configured in [None, Some(BEARER), Some(MAILBOX)] {
        let daemon = Daemon::spawn(Some(BEARER), configured);
        let body = daemon.body(json!({}));
        let mut request = headers(&body, "steward");
        request.remove(AUTHORIZATION);
        refusal(daemon.post(request, &body), 401, "unauthorized");
        refusal(
            daemon.post(headers(&body, "steward"), &body),
            503,
            "console-create-attestation-unavailable",
        );
        daemon.assert_counts(0, 0);
    }
    let daemon = Daemon::spawn(None, Some(TOKEN));
    let body = daemon.body(json!({}));
    refusal(
        daemon.post(headers(&body, "steward"), &body),
        503,
        "token-not-configured",
    );
    daemon.assert_counts(0, 0);
}

#[test]
fn attested_create_refuses_roles_identity_lineage_and_non_fresh_inputs() {
    let daemon = Daemon::spawn(Some(BEARER), Some(TOKEN));
    let body = daemon.body(json!({}));
    refusal(
        daemon.post(headers(&body, "operator"), &body),
        403,
        "console-create-role-forbidden",
    );
    for (extra, code) in [
        (json!({"lineage": {"depth": 0}}), "role-requires-root"),
        (
            json!({"lineage": {"forge_context": null}}),
            "identity_session_binding_untrusted",
        ),
        (
            json!({"attested_role": "steward"}),
            "identity_session_binding_untrusted",
        ),
        (json!({"role": null}), "identity_session_binding_untrusted"),
        (
            json!({"provider_resume_id": "fixture"}),
            "resume-unsupported",
        ),
        (json!({"id": null}), "console-create-invalid-body"),
        (json!({"id": "../invalid"}), "console-create-invalid-body"),
    ] {
        let body = daemon.body(extra);
        refusal(daemon.post(headers(&body, "steward"), &body), 400, code);
    }
    let body = br#"{"id":"fixture-steward","work":{"program":null,"program":null}}"#;
    refusal(
        daemon.post(headers(body, "steward"), body),
        400,
        "console-create-invalid-body",
    );
    let body = daemon.body(json!({"role": "steward", "lineage": {"forge_context": null}}));
    let mut request = headers(&body, "steward");
    request.remove(TOKEN_HEADER);
    request.remove(BINDING_HEADER);
    request.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    refusal(
        daemon.post(request, &body),
        400,
        "identity_session_binding_untrusted",
    );
    daemon.assert_counts(0, 0);
}

#[test]
fn attested_create_persists_broker_identity_and_fences_exact_retry() {
    let daemon = Daemon::spawn(Some(BEARER), Some(TOKEN));
    let body = daemon.body(json!({"lineage": {}}));
    let (status, response) = daemon.post(headers(&body, "steward"), &body);
    assert_eq!(status, 201, "{response}");
    let response: Value = serde_json::from_str(&response).unwrap();
    let session = &response["data"]["session"];
    let launch =
        json!({"initiator": "console-relay", "role": "steward", "attested_role": "steward"});
    assert_eq!(session["id"], "fixture-steward");
    assert_eq!(session["role"], "steward");
    assert_eq!(session["lineage"]["forge_context"], launch);
    assert_eq!(session["lineage"]["parent"], Value::Null);
    assert_eq!(session["lineage"]["depth"], 0);
    assert_eq!(session["lineage"]["root"]["machine"], MACHINE);
    assert_eq!(session["lineage"]["root"]["session_id"], "fixture-steward");
    assert_eq!(
        session["lineage"]["starter"],
        json!({"kind": "console", "via": "console"})
    );
    let record = daemon.record("fixture-steward");
    assert_eq!(record["role"], "steward");
    assert_eq!(record["lineage"], session["lineage"]);
    assert!(!record.to_string().contains(TOKEN));
    assert!(!record.to_string().contains("fixture-dispatcher"));
    let incarnation = session["session_incarnation"].as_str().unwrap();
    let binding = daemon.broker_identity(incarnation);
    assert_eq!(binding["session_incarnation"], incarnation);
    assert_eq!(binding["initiator"], "console-relay");
    assert_eq!(binding["attested_role"], "steward");
    let retry = daemon.post(headers(&body, "steward"), &body);
    refusal(retry, 409, "session-exists");
    assert_eq!(
        daemon.record("fixture-steward")["runtime"]["launch_id"],
        incarnation
    );
    daemon.assert_counts(1, 1);
}

#[test]
fn attested_create_does_not_grant_attestation_to_an_ordinary_root_role() {
    let daemon = Daemon::spawn(Some(BEARER), Some(TOKEN));
    let body = daemon.body(json!({"role": "steward"}));
    let mut request = headers(&body, "steward");
    request.remove(TOKEN_HEADER);
    request.remove(BINDING_HEADER);
    request.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let (status, response) = daemon.post(request, &body);
    assert_eq!(status, 200, "{response}");
    let response: Value = serde_json::from_str(&response).unwrap();
    let session = &response["data"]["session"];
    assert_eq!(session["role"], "steward");
    assert!(session["lineage"].get("forge_context").is_none());
    assert!(
        daemon.record("fixture-steward")["lineage"]
            .get("forge_context")
            .is_none()
    );
    daemon.assert_counts(1, 1);
}

#[test]
fn attested_create_reserves_the_console_relay_initiator_from_operator_selection() {
    let root = ScopedTempDir::with_prefix("attested-initiator-");
    let output = Command::new(nils_test_support::bin::resolve("agent-session"))
        .env_clear()
        .env("HOME", root.path())
        .arg("--state-dir")
        .arg(root.path().join("state"))
        .args([
            "start",
            "--agent",
            "claude",
            "--role",
            "steward",
            "--attest-role",
            "--forge-initiator",
            "console-relay",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "identity_session_binding_invalid");
}
