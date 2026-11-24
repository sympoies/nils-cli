use super::{Fixture, unread};
use futures_util::{SinkExt, StreamExt};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::{fs, thread};

use crate::cli::run;
use sha2::{Digest, Sha256};
use std::os::{fd::AsRawFd, unix::process::ExitStatusExt};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_tungstenite::tungstenite::Message;

const ID: &str = "guarded-delete";
const THREAD: &str = "fixture-conversation";

struct Provider {
    status: Arc<Mutex<&'static str>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
    _lease: fs::File,
}

impl Provider {
    fn start(fixture: &mut Fixture) -> Self {
        let runtime = fixture.record["runtime"]["launch_id"].as_str().unwrap();
        let runtime_dir = fixture._tmp.path().join("agent-session");
        fs::create_dir(&runtime_dir).unwrap();
        let digest = Sha256::digest(format!("{}\0{ID}\0{runtime}", fixture.state.display()));
        let name: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let socket = runtime_dir.join(format!("cx-{name}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let marker = fixture.session.join(".codex-app-server-idle-retirement");
        fs::write(
            &marker,
            json!({
                "schema_version": "agent-session.codex-idle-retirement.v2",
                "launch_id": runtime,
                "owner_pid": std::process::id(),
                "expires_at_epoch_ms": SystemTime::now()
                    .duration_since(UNIX_EPOCH).unwrap().as_millis() + 60_000,
                "token": uuid::Uuid::new_v4().to_string()
            })
            .to_string(),
        )
        .unwrap();
        let lease = fs::File::open(marker).unwrap();
        assert_eq!(unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_SH) }, 0);

        let binding: String = Sha256::digest(format!("agent-session-codex-thread-v1\0{THREAD}"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        fs::write(socket.with_extension("attached"), binding).unwrap();
        fixture.record["agent"] = json!("codex");
        fixture.record["provider_resume"]["provider"] = json!("codex");
        fixture.record["provider_resume"]["session_id"] = json!(THREAD);
        fixture.record["provider_resume"]["resume_args"] = json!(["resume", THREAD]);
        let fields = fixture.record["runtime"].as_object_mut().unwrap();
        fields.extend(
            json!({
                "kind": "codex_app_server",
                "codex_app_server_protocol": "v2",
                "codex_app_server_socket": socket,
                "codex_app_server_proxy": socket.with_extension("proxy"),
                "codex_app_server_thread_handoff": socket.with_extension("thread"),
                "codex_app_server_thread_attached": socket.with_extension("attached"),
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        write_record(fixture);
        fs::write(&fixture.name_status, "running").unwrap();
        fs::write(fixture._tmp.path().join("composer"), "› \n\n  status\n").unwrap();
        write_activity(fixture, "waiting");

        let status = Arc::new(Mutex::new("idle"));
        let observed_status = Arc::clone(&status);
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let listener = tokio::net::UnixListener::from_std(listener).unwrap();
                    loop {
                        let stream = tokio::select! {
                            _ = &mut stopped => break,
                            accepted = listener.accept() => accepted.unwrap().0,
                        };
                        let status = Arc::clone(&observed_status);
                        tokio::spawn(async move {
                            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                            while let Some(Ok(Message::Text(text))) = socket.next().await {
                                let request: Value = serde_json::from_str(&text).unwrap();
                                let Some(id) = request.get("id") else { continue };
                                let result = match request["method"].as_str().unwrap() {
                                    "initialize" => json!({}),
                                    "thread/loaded/list" => json!({"data": [THREAD], "nextCursor": null}),
                                    "thread/read" => {
                                        assert_eq!(request["params"]["threadId"], THREAD);
                                        json!({"thread": {"id": THREAD, "status": {"type": *status.lock().unwrap()}}})
                                    }
                                    method => panic!("idle retirement issued unexpected provider method {method}"),
                                };
                                if socket.send(Message::Text(json!({"id": id, "result": result}).to_string().into())).await.is_err() {
                                    break;
                                }
                            }
                        });
                    }
                });
        });
        Self {
            status,
            stop: Some(stop),
            worker: Some(worker),
            _lease: lease,
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.worker.take().unwrap().join().unwrap();
    }
}

fn write_record(fixture: &Fixture) {
    fs::write(
        fixture.session.join("session.json"),
        serde_json::to_vec(&fixture.record).unwrap(),
    )
    .unwrap();
}

fn write_activity(fixture: &Fixture, phase: &str) {
    fs::write(
        fixture.session.join("activity.json"),
        json!({
            "schema_version": "agent-session.activity.v1",
            "runtime_id": incarnation(fixture),
            "runtime_generation": 1,
            "seen_event_count": 0,
            "state": {
                "schema_version": "agent-session.turn-state.v1",
                "phase": phase, "phase_changed_at": jiff::Timestamp::now().to_string(),
                "revision": 2,
                "source": {"kind": "provider_hook", "confidence": "authoritative"}
            }
        })
        .to_string(),
    )
    .unwrap();
}

fn incarnation(fixture: &Fixture) -> &str {
    fixture.record["runtime"]["launch_id"].as_str().unwrap()
}

fn snapshot(fixture: &Fixture) -> Value {
    let output = run(
        fixture._tmp.path(),
        &[
            "--state-dir",
            fixture.state.to_str().unwrap(),
            "list",
            "--format",
            "json",
        ],
        &[("AGENT_SESSION_TMUX_BIN", fixture.tmux.to_str().unwrap())],
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    output.stdout_json()["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == ID)
        .unwrap()
        .clone()
}

fn delete(fixture: &Fixture, guard: &[&str]) -> nils_test_support::cmd::CmdOutput {
    delete_at(fixture._tmp.path(), &fixture.state, &fixture.tmux, guard)
}

fn delete_at(
    cwd: &Path,
    state: &Path,
    tmux: &Path,
    guard: &[&str],
) -> nils_test_support::cmd::CmdOutput {
    let mut args = vec![
        "--state-dir",
        state.to_str().unwrap(),
        "delete",
        ID,
        "--tmux-bin",
        tmux.to_str().unwrap(),
        "--format",
        "json",
    ];
    args.extend_from_slice(guard);
    run(cwd, &args, &[])
}

fn assert_refused(fixture: &mut Fixture, guard: &[&str], code: &str) -> Value {
    let record = fixture.session.join("session.json");
    let before = fs::read(&record).unwrap();
    let output = delete(fixture, guard);
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    let error = output.stdout_json()["error"].clone();
    assert_eq!(error["code"], code, "{}", output.stdout_text());
    assert_eq!(fs::read(&record).unwrap(), before);
    assert!(!fixture.killed.exists());
    assert!(fixture.pane.0.try_wait().unwrap().is_none());
    error
}

#[test]
fn idle_delete_requires_an_incarnation_and_excludes_stopped_mode() {
    let mut fixture = Fixture::new();
    let runtime = incarnation(&fixture).to_owned();
    let error = assert_refused(&mut fixture, &["--if-idle"], "parse-error");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("--expected-incarnation"),
        "{error}"
    );
    let error = assert_refused(
        &mut fixture,
        &[
            "--if-idle",
            "--if-stopped",
            "--expected-incarnation",
            &runtime,
        ],
        "parse-error",
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("cannot be used with"),
        "{error}"
    );
}

#[test]
fn idle_delete_retires_the_verified_runtime_and_record() {
    let mut fixture = Fixture::new();
    let _provider = Provider::start(&mut fixture);
    // Exercise the real ownership checks without a Linux cgroup or a real agent.
    fixture.record["runtime"]["agent_profile"] = json!("synthetic-codex");
    fixture.record["runtime"]["agent_profile_graceful_shutdown"] = json!("double-ctrl-c");
    write_record(&fixture);
    let runtime = incarnation(&fixture).to_owned();
    let guard = [
        "--if-idle",
        "--expected-incarnation",
        &runtime,
        "--if-unread-empty",
    ];
    let cwd = fixture._tmp.path();
    let (state, tmux) = (&fixture.state, &fixture.tmux);
    let output = thread::scope(|scope| {
        let delete = scope.spawn(|| delete_at(cwd, state, tmux, &guard));
        // The fixture owns the pane child. Reap it while the CLI verifies
        // that its process group exited, as a real tmux parent would.
        while !delete.is_finished() && fixture.pane.0.try_wait().unwrap().is_none() {
            thread::sleep(Duration::from_millis(10));
        }
        delete.join().unwrap()
    });
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    let receipt = &output.stdout_json()["data"];
    assert_eq!(receipt["id"], ID);
    assert_eq!(receipt["killed"], true);
    assert_eq!(receipt["deleted"], true);
    assert!(fixture.killed.exists());
    assert!(!fixture.session.exists());
    assert_eq!(fixture.pane.0.wait().unwrap().signal(), Some(libc::SIGINT));
}

#[test]
fn idle_delete_refuses_replacement_after_caller_observes_incarnation() {
    let mut fixture = Fixture::new();
    let _provider = Provider::start(&mut fixture);
    let observed = snapshot(&fixture);
    let expected = observed["session_incarnation"].as_str().unwrap();
    assert_eq!(expected, incarnation(&fixture));
    fixture.record["runtime"]["launch_id"] = json!("replacement-runtime");
    fixture.record["runtime"]["generation"] = json!(2);
    write_record(&fixture);
    assert_refused(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", expected],
        "session-incarnation-conflict",
    );
}

#[test]
fn idle_delete_rechecks_provider_after_caller_observes_waiting() {
    let mut fixture = Fixture::new();
    let provider = Provider::start(&mut fixture);
    let observed = snapshot(&fixture);
    assert_eq!(observed["status"], "running");
    assert_eq!(observed["turn_state"]["phase"], "waiting");
    let runtime = incarnation(&fixture).to_owned();
    let guard = ["--if-idle", "--expected-incarnation", &runtime];
    // The published activity is still waiting while the provider has begun work.
    *provider.status.lock().unwrap() = "active";
    assert_refused(&mut fixture, &guard, "session-state-mismatch");
    *provider.status.lock().unwrap() = "notLoaded";
    assert_refused(&mut fixture, &guard, "session-status-unknown");
    *provider.status.lock().unwrap() = "idle";
    write_activity(&fixture, "working");
    assert_refused(&mut fixture, &guard, "session-state-mismatch");
    write_activity(&fixture, "unknown");
    assert_refused(&mut fixture, &guard, "session-status-unknown");
}

#[test]
fn idle_delete_rechecks_mail_delivered_after_an_empty_snapshot() {
    let mut fixture = Fixture::new();
    let _provider = Provider::start(&mut fixture);
    let runtime = incarnation(&fixture).to_owned();
    unread::seed_mailbox(&fixture, &runtime);
    write_activity(&fixture, "waiting");
    let observed = snapshot(&fixture);
    assert_eq!(observed["unread_message_count"], 0);
    let sent = unread::deliver(&fixture);
    assert_eq!(sent.code, 0, "{}", sent.stdout_text());
    let guard = [
        "--if-idle",
        "--expected-incarnation",
        &runtime,
        "--if-unread-empty",
    ];
    let error = assert_refused(&mut fixture, &guard, "session-mailbox-not-empty");
    assert_eq!(error["details"]["unread_message_count"], 1);
    fs::write(fixture.state.join("coordination/registry.json"), "{invalid").unwrap();
    assert_refused(&mut fixture, &guard, "session-mailbox-unknown");
}

#[test]
fn idle_delete_refuses_a_draft_missing_composer_or_pending_ingress() {
    let mut fixture = Fixture::new();
    let _provider = Provider::start(&mut fixture);
    let runtime = incarnation(&fixture).to_owned();
    let guard = ["--if-idle", "--expected-incarnation", &runtime];
    let composer = fixture._tmp.path().join("composer");
    fs::write(&composer, "› owner draft\n\n  status\n").unwrap();
    assert_refused(&mut fixture, &guard, "send-composer-not-empty");
    assert_eq!(
        fs::read_to_string(&composer).unwrap(),
        "› owner draft\n\n  status\n"
    );
    fs::write(&composer, "screen without a composer\n").unwrap();
    assert_refused(&mut fixture, &guard, "send-composer-unobservable");

    let pending = fs::File::create(fixture.session.join(".activity-ingress.lock")).unwrap();
    assert_eq!(
        unsafe { libc::flock(pending.as_raw_fd(), libc::LOCK_SH) },
        0
    );
    fs::write(&composer, "› \n\n  status\n").unwrap();
    assert_refused(&mut fixture, &guard, "session-state-mismatch");
    drop(pending);

    let activity = fixture.session.join("activity.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&activity).unwrap()).unwrap();
    document["pending_attention"] = json!([{
        "id": "fixture-attention",
        "kind": "question",
        "requested_at": jiff::Timestamp::now().to_string(),
    }]);
    fs::write(&activity, document.to_string()).unwrap();
    assert_refused(&mut fixture, &guard, "session-state-mismatch");
}

#[test]
fn idle_delete_refuses_unfenced_runtime_or_unverified_liveness() {
    let mut fixture = Fixture::new();
    let runtime = incarnation(&fixture).to_owned();
    let guard = ["--if-idle", "--expected-incarnation", &runtime];
    fs::write(&fixture.name_status, "running").unwrap();
    assert_refused(&mut fixture, &guard, "session-idle-delete-unsupported");
    let provider = Provider::start(&mut fixture);
    fs::write(&fixture.name_status, "stopped").unwrap();
    assert_refused(&mut fixture, &guard, "session-state-mismatch");
    fs::write(&fixture.name_status, "unknown").unwrap();
    assert_refused(&mut fixture, &guard, "session-status-unknown");
    drop(provider);

    // A supported record shape and an unlocked capability left behind by
    // a dead or upgraded proxy cannot authorize guarded teardown.
    fs::write(&fixture.name_status, "running").unwrap();
    assert_refused(&mut fixture, &guard, "session-idle-delete-unsupported");
}

#[test]
fn idle_delete_refuses_a_compaction_in_flight() {
    let mut fixture = Fixture::new();
    let _provider = Provider::start(&mut fixture);
    let runtime = incarnation(&fixture).to_owned();
    let compaction = fs::File::create(
        fixture
            .session
            .join(".codex-app-server-retirement-requests.lock"),
    )
    .unwrap();
    assert_eq!(
        unsafe { libc::flock(compaction.as_raw_fd(), libc::LOCK_SH) },
        0
    );
    assert_refused(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", &runtime],
        "session-state-mismatch",
    );
}
