use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use agent_session::cli::resume_move::StopProof;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use pretty_assertions::assert_eq;
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde_json::{Value, json};

use super::claude::with_console_fixture;
use super::support::{Signers, write_private};

const BEARER: &str = "synthetic-machine-bearer";
const INGRESS: &str = "synthetic-relay-ingress-token-000001";
const SCHEMA: &str = "agent-session.console-resume-move.v1";
const STATUS_SCHEMA: &str = "agent-session.console-resume-move-status.v1";
const BINDING_HEADER: &str = "X-Agent-Session-Resume-Move-Binding";
const INGRESS_HEADER: &str = "X-Agent-Session-Relay-Token";
const START: &str = "/sessions/resume-move/v1";
const OPERATION: &str = "01900000-0000-7000-8000-000000000020";
const CALLER_INCARNATION: &str = "00000000-0000-4000-8000-000000000030";

pub(super) struct Fixture<'a> {
    pub root: &'a Path,
    pub state: &'a Path,
    pub cwd: &'a Path,
    pub tmux: &'a Path,
    pub tmux_log: &'a Path,
    pub broker: &'a Path,
    pub staged_history: &'a Path,
    pub provider_bin: &'a Path,
    pub signers: &'a Signers,
    pub proof: &'a StopProof,
    pub ssh_keygen: &'a Path,
}

impl Fixture<'_> {
    fn daemon(&self, ingress: bool, allowlist: Option<&Path>, fail_launch: bool) -> Daemon {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut command = crate::serve_fixture::command(
            self.root,
            nils_test_support::bin::resolve("agent-session"),
            self.state,
            self.tmux,
            |command| {
                command
                    .args([
                        "--bind",
                        &address.to_string(),
                        "--machine",
                        "destination",
                        "--token",
                        BEARER,
                    ])
                    .env("AGENT_SESSION_RESUME_MOVE_HOST", "host-b")
                    .env("AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN", self.ssh_keygen)
                    .env(
                        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
                        json!([self.broker]).to_string(),
                    )
                    .env("AGENT_SESSION_CLAUDE_BIN", self.provider_bin)
                    .env("AGENT_SESSION_FAKE_TMUX_LOG", self.tmux_log)
                    .env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit());
                if ingress {
                    command
                        .env("AGENT_SESSION_RELAY_URL", "http://127.0.0.1:1")
                        .env(
                            "AGENT_SESSION_RELAY_TOKEN",
                            "synthetic-relay-outbound-token-000001",
                        )
                        .env("AGENT_SESSION_RELAY_INGRESS_TOKEN", INGRESS);
                }
                if let Some(path) = allowlist {
                    command.env("AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE", path);
                }
                if fail_launch {
                    command.env("AGENT_SESSION_FAKE_TMUX_FAIL", "new-session");
                }
            },
        );
        let mut daemon = Daemon {
            child: command.spawn().unwrap(),
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap(),
            base: format!("http://{address}"),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(address).is_err() {
            assert!(daemon.child.try_wait().unwrap().is_none());
            assert!(
                Instant::now() < deadline,
                "destination daemon did not listen"
            );
            thread::sleep(Duration::from_millis(10));
        }
        daemon
    }

    fn move_id(&self) -> &str {
        &self.proof.binding.move_id
    }

    fn request(&self) -> Value {
        json!({
            "schema_version": SCHEMA, "operation_id": OPERATION, "machine": "destination",
            "caller": {"session_id": "caller", "session_incarnation": CALLER_INCARNATION},
            "session": {
                "move_id": self.move_id(), "agent": "claude",
                "resume": self.proof.binding.provider_session_id, "account": "fixture-account",
                "cwd": self.cwd
            }
        })
    }

    fn binding(&self, scope: &str) -> Value {
        let mut binding = json!({
            "schema_version": "agent-session.console-resume-move-binding.v1",
            "origin_machine": "origin", "caller_session_id": "caller",
            "caller_session_incarnation": CALLER_INCARNATION, "owner": "fixture-owner",
            "destination_machine": "destination", "move_id": self.move_id(), "scope": scope
        });
        if scope == "start" {
            binding["operation_id"] = json!(OPERATION);
        }
        binding
    }

    fn status_path(&self) -> String {
        format!("/sessions/resume-move/{}/status/v1", self.move_id())
    }

    fn ledger_path(&self) -> PathBuf {
        self.state
            .join("coordination/resume-move-destination.v1.json")
    }

    fn ledger(&self) -> Vec<u8> {
        fs::read(self.ledger_path()).unwrap()
    }

    fn launch_count(&self) -> usize {
        super::super::cli::tmux_calls(self.tmux_log)
            .iter()
            .filter(|call| call.first().is_some_and(|arg| arg == "new-session"))
            .count()
    }
}

struct Daemon {
    child: Child,
    client: Client,
    base: String,
}

impl Daemon {
    fn get(&self, path: &str) -> RequestBuilder {
        self.client.get(format!("{}{path}", self.base))
    }

    fn start(&self, body: Vec<u8>) -> RequestBuilder {
        self.client
            .post(format!("{}{START}", self.base))
            .header("Content-Type", "application/json")
            .body(body)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            // SAFETY: the still-owned fixture daemon handles SIGINT by draining.
            unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGINT) };
            let _ = self.child.wait();
        }
    }
}

fn bind(request: RequestBuilder, binding: &Value) -> RequestBuilder {
    request
        .bearer_auth(BEARER)
        .header(INGRESS_HEADER, INGRESS)
        .header(
            BINDING_HEADER,
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(binding).unwrap()),
        )
}

fn response(reply: Response, status: u16) -> Value {
    let actual = reply.status();
    let body = reply.text().unwrap();
    assert_eq!(actual.as_u16(), status, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn refusal(schema: &str, code: &str, details: Value) -> Value {
    json!({"schema_version":schema,"ok":false,"error":{"code":code,"details":details}})
}

#[test]
fn console_destination_start_replays_and_reports_read_only_status() {
    with_console_fixture(|f| {
        let daemon = f.daemon(true, Some(&f.signers.allowlist), false);
        let status = || {
            bind(daemon.get(&f.status_path()), &f.binding("read"))
                .send()
                .unwrap()
        };
        let start = |body| {
            bind(daemon.start(body), &f.binding("start"))
                .send()
                .unwrap()
        };
        let before = f.ledger();
        let control = fs::OpenOptions::new()
            .write(true)
            .open(f.state.join("coordination/resume-move-destination.lock"))
            .unwrap();
        control.lock().unwrap();
        let staged = status();
        assert_eq!(staged.headers()["cache-control"], "no-store");
        assert_eq!(
            response(staged, 200),
            json!({"schema_version":STATUS_SCHEMA,"ok":true,"data":{
                "machine":"destination","move_id":f.move_id(),"state":"staged",
                "session_id":null,"session_incarnation":null,"proof_freshness":"fresh",
                "proof_expires_at_epoch":f.proof.expires_at
            }})
        );
        assert_eq!(f.ledger(), before);
        assert_eq!(f.launch_count(), 0);
        drop(control);

        // The relay forwards the original complete JSON bytes, including
        // whitespace and the caller-supplied operation UUID.
        let bytes = serde_json::to_vec_pretty(&f.request()).unwrap();
        let created = response(start(bytes.clone()), 200);
        let id = format!("move-{}", f.move_id());
        let session = f.state.join("sessions").join(&id);
        let record = || {
            let bytes = fs::read(session.join("session.json")).unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        };
        let incarnation = record()["runtime"]["launch_id"].clone();
        assert!(incarnation.is_string());
        assert_eq!(
            created,
            json!({"schema_version":SCHEMA,"ok":true,"data":{
                "machine":"destination","move_id":f.move_id(),"state":"created",
                "session_id":id,"session_incarnation":incarnation
            }})
        );
        assert_eq!(response(start(bytes), 200), created);
        let after = f.ledger();
        let dirs = || {
            fs::read_dir(f.state.join("sessions"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>()
        };
        let sessions_before = dirs();
        assert_eq!(sessions_before.len(), 1);
        assert_eq!(
            response(status(), 200)["data"]["session_incarnation"],
            incarnation
        );
        assert_eq!(dirs(), sessions_before);
        assert_eq!(f.ledger(), after);
        assert_eq!(f.launch_count(), 1);
        let lineage = json!({
            "schema_version":"agent-session.resume-move-lineage.v1",
            "move_id":f.move_id(),"source_host":f.proof.binding.source_host,
            "source_session_id":f.proof.binding.source_session_id,
            "source_incarnation":f.proof.binding.source_incarnation,
            "source_generation":f.proof.source_generation,"provider":"claude",
            "provider_session_id":f.proof.binding.provider_session_id
        });
        assert_eq!(record()["resume_move"], lineage);
        for (token, expected) in [
            (None, None),
            (Some("wrong-bearer"), None),
            (Some(BEARER), Some(&lineage)),
        ] {
            let mut request = daemon.get("/sessions");
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let listing = response(request.send().unwrap(), 200);
            let listed = listing["data"]["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|session| session["id"] == id)
                .unwrap();
            assert_eq!(listed.get("resume_move"), expected);
        }

        let mut expired: StopProof = serde_json::from_value(json!(f.proof)).unwrap();
        expired.issued_at -= 10;
        expired.expires_at = f.proof.issued_at - 1;
        let proof_dir = f.root.join("expired-proof");
        fs::create_dir(&proof_dir).unwrap();
        let signed: Value =
            serde_json::from_slice(&fs::read(f.signers.sign_proof(&proof_dir, &expired)).unwrap())
                .unwrap();
        let mut ledger: Value = serde_json::from_slice(&after).unwrap();
        let expected = [
            (signed.clone(), "expired", json!(expired.expires_at)),
            (
                json!({"payload": signed["payload"], "signature": "invalid"}),
                "unverified",
                Value::Null,
            ),
            (Value::Null, "missing", Value::Null),
        ];
        for (proof, freshness, expiry) in expected {
            ledger[f.move_id()]["proof"] = proof;
            let bytes = serde_json::to_vec(&ledger).unwrap();
            write_private(&f.ledger_path(), &bytes);
            let read = response(status(), 200);
            assert_eq!(read["data"]["state"], "created");
            assert_eq!(read["data"]["session_incarnation"], incarnation);
            assert_eq!(read["data"]["proof_freshness"], freshness);
            assert_eq!(read["data"]["proof_expires_at_epoch"], expiry);
            assert_eq!(f.ledger(), bytes);
        }
    });
}

#[test]
fn console_destination_session_list_prevents_storage_for_every_bearer() {
    with_console_fixture(|f| {
        let daemon = f.daemon(true, Some(&f.signers.allowlist), false);
        for token in [None, Some("wrong-bearer"), Some(BEARER)] {
            let mut request = daemon.get("/sessions");
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let reply = request.send().unwrap();
            assert_eq!(reply.status(), reqwest::StatusCode::OK);
            assert_eq!(
                reply.headers().get(reqwest::header::CACHE_CONTROL),
                Some(&reqwest::header::HeaderValue::from_static("no-store")),
                "session listing must prohibit storage for bearer {token:?}"
            );
        }
    });
}

#[test]
fn console_destination_capability_requires_ingress_and_native_paths() {
    with_console_fixture(|f| {
        let bad = f.root.join("absent-signers");
        for (ingress, trust, expected) in [
            (false, Some(f.signers.allowlist.as_path()), false),
            (true, None, false),
            (true, Some(bad.as_path()), false),
            (true, Some(f.signers.allowlist.as_path()), true),
        ] {
            let daemon = f.daemon(ingress, trust, false);
            let listing = response(daemon.get("/sessions").send().unwrap(), 200);
            assert_eq!(
                listing["data"]["capabilities"]["resume_move_start_v1"],
                expected
            );
        }
    });
}

#[test]
fn console_destination_refusals_authenticate_before_exposing_move() {
    with_console_fixture(|f| {
        let daemon = f.daemon(true, Some(&f.signers.allowlist), false);
        let request = f.request();
        let start = |body: &Value| daemon.start(serde_json::to_vec(body).unwrap());
        let details = || json!({"safe_to_retry":false,"next_action":"inspect_destination","session_created":false});
        let before = f.ledger();
        let duplicate = bind(start(&request), &f.binding("start")).bearer_auth(BEARER);
        for unauthed in [
            start(&request).header(INGRESS_HEADER, INGRESS),
            start(&request)
                .bearer_auth("wrong")
                .header(INGRESS_HEADER, INGRESS),
            start(&request).bearer_auth(BEARER),
            start(&request)
                .bearer_auth(BEARER)
                .header(INGRESS_HEADER, "wrong"),
            duplicate,
            bind(start(&request), &f.binding("start")).header(INGRESS_HEADER, INGRESS),
        ] {
            assert_eq!(
                response(unauthed.send().unwrap(), 401),
                refusal(SCHEMA, "resume-move-unavailable", details())
            );
        }
        let mut old_body = request.clone();
        old_body["caller"]["owner"] = json!("fixture-owner");
        let mut mismatched_operation = f.binding("start");
        mismatched_operation["operation_id"] = json!("01900000-0000-7000-8000-000000000021");
        let mut wrong_machine = f.binding("start");
        wrong_machine["destination_machine"] = json!("elsewhere");
        let bare_binding = || {
            start(&request)
                .bearer_auth(BEARER)
                .header(INGRESS_HEADER, INGRESS)
        };
        for invalid_binding in [
            bare_binding(),
            bare_binding().header(BINDING_HEADER, "not base64url"),
            bare_binding().header(BINDING_HEADER, "a".repeat(2049)),
            bind(start(&request), &f.binding("read")),
            bind(start(&request), &wrong_machine),
            bind(start(&request), &mismatched_operation),
            bind(start(&old_body), &f.binding("start")),
            bind(start(&request), &f.binding("start")).header(BINDING_HEADER, "duplicate"),
        ] {
            assert_eq!(
                response(invalid_binding.send().unwrap(), 403),
                refusal(SCHEMA, "resume-move-binding-mismatch", details())
            );
        }
        let duplicate_body = serde_json::to_string(&request).unwrap().replacen(
            "\"agent\":\"claude\"",
            "\"agent\":\"claude\",\"agent\":\"claude\"",
            1,
        );
        assert_eq!(
            response(
                bind(
                    daemon.start(duplicate_body.into_bytes()),
                    &f.binding("start")
                )
                .send()
                .unwrap(),
                400
            ),
            refusal(SCHEMA, "resume-unsupported", details())
        );
        let mut wrong_account = request.clone();
        wrong_account["session"]["account"] = json!("wrong-account");
        let mut wrong_path = request;
        wrong_path["session"]["cwd"] = json!(f.root);
        for (bad, code) in [
            (wrong_account, "resume-account-mismatch"),
            (wrong_path, "resume-move-path-mismatch"),
        ] {
            assert_eq!(
                response(bind(start(&bad), &f.binding("start")).send().unwrap(), 422),
                refusal(
                    SCHEMA,
                    code,
                    json!({
                        "move_id":f.move_id(),"state":"staged","safe_to_retry":false,
                        "session_created":false,"next_action":"inspect_destination"
                    })
                )
            );
        }
        let mut invalid_read_binding = f.binding("read");
        invalid_read_binding["operation_id"] = Value::Null;
        let invalid_read = bind(daemon.get(&f.status_path()), &invalid_read_binding);
        assert_eq!(
            response(invalid_read.send().unwrap(), 403),
            refusal(
                STATUS_SCHEMA,
                "resume-move-binding-mismatch",
                json!({
                    "safe_to_retry":false,"next_action":"inspect_destination"
                })
            )
        );
        let missing_id = "00000000-0000-4000-8000-000000000021";
        let mut missing_binding = f.binding("read");
        missing_binding["move_id"] = json!(missing_id);
        let missing_path = format!("/sessions/resume-move/{missing_id}/status/v1");
        assert_eq!(
            response(
                bind(daemon.get(&missing_path), &missing_binding)
                    .send()
                    .unwrap(),
                404
            ),
            refusal(
                STATUS_SCHEMA,
                "resume-move-proof-missing",
                json!({
                    "move_id":missing_id,"safe_to_retry":false,"next_action":"inspect_destination"
                })
            )
        );
        let query = format!("{}?selector=extra", f.status_path());
        let refused = bind(daemon.get(&query), &f.binding("read")).send().unwrap();
        assert_eq!(refused.headers()["cache-control"], "no-store");
        assert_eq!(
            response(refused, 400),
            refusal(
                STATUS_SCHEMA,
                "resume-unsupported",
                json!({
                    "safe_to_retry":false,"next_action":"inspect_destination"
                })
            )
        );
        assert_eq!(f.ledger(), before);
        assert_eq!(f.launch_count(), 0);
    });
}

#[test]
fn console_destination_unknown_outcome_never_launches_again() {
    with_console_fixture(|f| {
        let daemon = f.daemon(true, Some(&f.signers.allowlist), true);
        let body = serde_json::to_vec(&f.request()).unwrap();
        let start = |daemon: &Daemon| {
            bind(daemon.start(body.clone()), &f.binding("start"))
                .send()
                .unwrap()
        };
        let unknown = refusal(
            SCHEMA,
            "resume-move-outcome-unknown",
            json!({
                "move_id":f.move_id(),"state":"writer_admitted",
                "safe_to_retry":false,"next_action":"reconcile_destination"
            }),
        );
        assert_eq!(response(start(&daemon), 502), unknown);
        let admitted = f.ledger();
        drop(daemon);
        let daemon = f.daemon(true, Some(&f.signers.allowlist), false);
        let read = bind(daemon.get(&f.status_path()), &f.binding("read"))
            .send()
            .unwrap();
        assert_eq!(response(read, 200)["data"]["state"], "writer_admitted");
        assert_eq!(response(start(&daemon), 502), unknown);
        assert_eq!(f.ledger(), admitted);
        assert_eq!(f.launch_count(), 1);
    });
}

#[test]
fn console_destination_preserves_missing_staged_history_refusal() {
    with_console_fixture(|f| {
        let daemon = f.daemon(true, Some(&f.signers.allowlist), false);
        let before = f.ledger();
        fs::rename(f.staged_history, f.root.join("withheld-history")).unwrap();
        let request = daemon.start(serde_json::to_vec(&f.request()).unwrap());
        let refused = bind(request, &f.binding("start")).send().unwrap();
        assert_eq!(
            response(refused, 422),
            refusal(
                SCHEMA,
                "resume-history-missing",
                json!({
                    "move_id":f.move_id(),"state":"staged",
                    "next_action":"inspect_destination","safe_to_retry":false,
                    "session_created":false
                })
            )
        );
        assert_eq!(f.ledger(), before);
        assert_eq!(f.launch_count(), 0);
    });
}
