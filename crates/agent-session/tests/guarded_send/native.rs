//! Native RPC regressions retain independent modeled composer evidence.
use super::*;
use futures_util::{SinkExt, StreamExt};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message;

const GUARDS: &[&str] = &[
    "--require-idle",
    "--require-empty-composer",
    "--expect-runtime",
    "runtime-one",
    "--expect-provider-session",
    "provider-one",
    "--idempotency-key",
    "native-once",
];

pub(super) struct Server {
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub(super) fn start(fixture: &Fixture, mode: &'static str) -> Self {
        let listener =
            std::os::unix::net::UnixListener::bind(fixture.root().join("cx.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let digest = Sha256::digest(b"agent-session-codex-thread-v1\0provider-one");
        let binding = digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        fs::write(fixture.root().join("cx.attached"), binding).unwrap();
        let claim_dir = fixture.root().join("state/sessions/guarded");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::UnixListener::from_std(listener).unwrap();
                loop {
                    let stream = tokio::select! {
                        _ = &mut stopped => break,
                        accepted = listener.accept() => accepted.unwrap().0,
                    };
                    let observed = observed.clone();
                    let claim_dir = claim_dir.clone();
                    tokio::spawn(async move {
                        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                        while let Some(Ok(Message::Text(text))) = socket.next().await {
                            let request: Value = serde_json::from_str(&text).unwrap();
                            observed.lock().unwrap().push(request.clone());
                            let Some(id) = request.get("id") else { continue };
                            let method = request["method"].as_str().unwrap();
                            let result = match method {
                                "initialize" => json!({}),
                                "thread/loaded/list" => json!({"data":["provider-one"],"nextCursor":null}),
                                "thread/resume" => json!({"thread":{"id":"provider-one"}}),
                                "thread/read" => {
                                    let calls = observed.lock().unwrap();
                                    let submitted = calls.iter().find(|r| r["method"] == "turn/start");
                                    let read_count = calls.iter().filter(|r| r["method"] == "thread/read").count();
                                    let claimed = fs::read_dir(&claim_dir).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with("send-key-"));
                                    let busy = mode == "busy" || (mode == "late-busy" && read_count >= 3)
                                        || (mode == "busy-after-claim" && claimed);
                                    let submitted = if mode == "history-lag" && calls.iter().filter(|r| r["method"] == "thread/read" && r["params"]["includeTurns"] == true).count() == 1 { None } else { submitted };
                                    let turns = submitted.map_or(json!([]), |r| json!([{
                                        "id":"turn-one", "status":"inProgress", "items":[{
                                            "type":"userMessage", "id":"input-one", "content":[{
                                                "type":"text", "text":if mode == "wrong-input" {json!("other input")} else {r["params"]["input"][0]["text"].clone()}
                                            }]
                                        }]
                                    }]));
                                    json!({"thread":{"id":if mode == "wrong-thread" {"other-thread"} else {"provider-one"}, "status":{"type":if busy {"active"} else {"idle"}}, "turns":turns}})
                                }
                                "turn/start" => {
                                    if mode == "disconnect" { break; }
                                    json!({"turn":{"id":"turn-one","status":"inProgress","items":[]}})
                                }
                                "thread/compact/start" => json!({}),
                                _ => panic!("unexpected native method: {method}"),
                            };
                            if socket.send(Message::Text(json!({"id":id,"result":result}).to_string().into())).await.is_err() { break; }
                        }
                    });
                }
            });
        });
        Self {
            requests,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    fn history_reads(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| {
                request["method"] == "thread/read" && request["params"]["includeTurns"] == true
            })
            .count()
    }

    pub(super) fn mutations(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                matches!(
                    r["method"].as_str(),
                    Some("turn/start" | "thread/compact/start")
                )
            })
            .cloned()
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn native_idle_uses_thread_state_and_submits_once_without_paste() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    fixture.arm_recovery();
    fs::remove_file(fixture.root().join("state/sessions/guarded/activity.json")).unwrap();
    let output = fixture.send(GUARDS);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let value = output.stdout_json();
    assert_eq!(value["data"]["submission"]["outcome"], "submitted");
    assert_eq!(value["data"]["submission"]["enter_presses"], 0);
    assert_eq!(value["data"]["native"]["confirmation"], "input_observed");
    fixture.no_delivery();
    assert_eq!(server.mutations().len(), 1);
    assert_eq!(
        server.mutations()[0]["params"]["input"][0]["text"],
        "continue-fixture"
    );
    fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
    assert_eq!(server.mutations().len(), 1);
}

#[test]
fn native_busy_and_raced_turn_refuse_without_paste() {
    for mode in ["busy", "late-busy"] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, mode);
        let recovery = fixture.arm_recovery();
        fixture.refused(&fixture.send(GUARDS), "send-session-not-idle");
        fixture.unchanged_control_state(&recovery);
        assert!(server.mutations().is_empty());
    }
}

#[test]
fn native_unknown_and_drafted_composers_refuse_without_paste() {
    for draft in [None, Some("user draft"), Some(" ")] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, "idle");
        let recovery = fixture.arm_recovery();
        let reason = if let Some(draft) = draft {
            fs::write(fixture.root().join("composer"), draft).unwrap();
            "send-composer-not-empty"
        } else {
            fs::write(fixture.root().join("capture-fails"), "").unwrap();
            "send-composer-unobservable"
        };
        fixture.refused(&fixture.send(GUARDS), reason);
        fixture.unchanged_control_state(&recovery);
        assert!(server.mutations().is_empty());
    }
}

#[test]
fn native_compact_starts_compaction_without_prompt_or_paste() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    let output = fixture.send_text(Some("/compact\n"), GUARDS);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["native"]["confirmation"],
        "compact_started"
    );
    fixture.no_delivery();
    assert_eq!(
        server.mutations(),
        [
            json!({"id":server.mutations()[0]["id"],"method":"thread/compact/start","params":{"threadId":"provider-one"}})
        ]
    );
}

#[test]
fn native_uncertain_confirmation_retains_key_and_leaves_no_residue() {
    for mode in ["wrong-input", "disconnect"] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, mode);
        let value = fixture.refused(&fixture.send(GUARDS), "send-native-unconfirmed");
        assert_eq!(
            value["error"]["details"]["acknowledged"],
            mode == "wrong-input"
        );
        assert_eq!(
            value["error"]["details"]["submitted"],
            if mode == "wrong-input" {
                json!(true)
            } else {
                Value::Null
            }
        );
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            ""
        );
        assert_eq!(server.mutations().len(), 1);
        fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
        assert_eq!(server.mutations().len(), 1);
    }
}

#[test]
fn native_wrong_thread_refuses_before_input() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "wrong-thread");
    fixture.refused(&fixture.send(GUARDS), "send-identity-mismatch");
    fixture.no_delivery();
    assert!(server.mutations().is_empty());
}

#[test]
fn native_attention_cannot_be_overridden_by_allow_blocked() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    let recovery = fixture.arm_recovery();
    let mut activity = fixture.document("activity.json");
    activity["state"]["phase"] = json!("needs_input");
    fs::write(
        fixture.root().join("state/sessions/guarded/activity.json"),
        serde_json::to_vec(&activity).unwrap(),
    )
    .unwrap();
    let mut guards = GUARDS.to_vec();
    guards.push("--allow-blocked");
    fixture.refused(&fixture.send(&guards), "send-session-not-idle");
    fixture.unchanged_control_state(&recovery);
    assert!(server.mutations().is_empty());
}

#[test]
fn native_route_requires_both_explicit_continuation_guards() {
    for guard in ["--expect-runtime", "--idempotency-key", "--require-idle"] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(
            &fixture,
            if guard == "--require-idle" {
                "idle"
            } else {
                "busy"
            },
        );
        let extra: &[&str] = match guard {
            "--expect-runtime" => &[guard, "runtime-one"],
            "--idempotency-key" => &[guard, "partial-profile"],
            _ => &[guard],
        };
        let output = fixture.send(extra);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert!(output.stdout_json()["data"].get("native").is_none());
        assert!(server.mutations().is_empty());
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|c| c[0] == "paste-buffer")
                .count(),
            1
        );
    }
}

#[test]
fn native_unsupported_slash_command_refuses_without_paste() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    fixture.refused(
        &fixture.send_text(Some("/model"), GUARDS),
        "send-native-command-unsupported",
    );
    fixture.no_delivery();
    assert!(server.mutations().is_empty());
}

#[test]
fn native_waits_for_input_visibility_without_resubmission() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "history-lag");
    let output = fixture.send(GUARDS);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["native"]["confirmation"],
        "input_observed"
    );
    assert!(
        server.history_reads() >= 2,
        "success must follow a later input-bearing read"
    );
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
}

#[test]
fn native_turn_start_after_claim_refuses_dispatch_and_retains_key() {
    let fixture = Fixture::new("codex");
    let _lease = fixture.bind_codex();
    let server = Server::start(&fixture, "busy-after-claim");
    let recovery = fixture.arm_recovery();
    fixture.refused(&fixture.send(GUARDS), "send-session-not-idle");
    fixture.unchanged_control_state(&recovery);
    assert!(server.mutations().is_empty());
    fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
    assert!(server.mutations().is_empty());
}
