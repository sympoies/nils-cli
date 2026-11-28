use super::{Fixture, ID, assert_refused, delete_at, incarnation, unread, write_record};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, os::fd::AsRawFd, os::unix::process::ExitStatusExt, thread, time::Duration};

const EMPTY_COMPOSER: &str = "────────────────────\n❯ \n────────────────────\n";

fn claude_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    let cwd = fixture._tmp.path();
    crate::cli::write_executable(
        &cwd.join("claude"),
        "#!/bin/sh\n[ \"$*\" = 'agents --json' ] || exit 42\nexec cat claude-agents.json\n",
    );
    fs::write(
        cwd.join("claude-agents.json"),
        r#"[{"sessionId":"resume-session-id","status":"idle"}]"#,
    )
    .unwrap();
    fs::write(&fixture.name_status, "running").unwrap();
    fs::write(cwd.join("composer"), EMPTY_COMPOSER).unwrap();
    let tmux = fs::read_to_string(&fixture.tmux).unwrap().replace(
        " capture-pane)",
        r#" show-options)
   [ "$2:$3:$4:$5" = '-AHv:-t:%91:after-send-keys' ] || exit 42
   if [ -f shutdown-hook ]; then cat shutdown-hook; fi ;;
 select-pane)
   case "$2:$3:$4" in
     '-d:-t:%91') : > input-off; printf 'paused\n' >> input-events ;;
     '-e:-t:%91') rm -f input-off; printf 'resumed\n' >> input-events ;;
     *) exit 42 ;;
   esac ;;
 capture-pane)"#,
    );
    let tmux = tmux
        .replace(
            " display-message) ",
            r#" display-message) if [ "$5" = '#{pane_id} #{pane_input_off} #{socket_path}' ]; then printf '%%91 %s fixture.sock\n' "$([ -f input-off ] && echo 1 || echo 0)"; exit; fi; "#,
        )
        .replace(
            " if-shell)",
            r#" if-shell)
   if grep -q '^paused$' input-events; then
     [ "$6" = 'select-pane -e -t %91 ; send-keys -t %91 C-c ; select-pane -d -t %91' ] || exit 42
   fi"#,
        );
    fs::write(&fixture.tmux, tmux).unwrap();
    fixture.record["runtime"]["agent_profile"] = json!("synthetic-claude");
    fixture.record["runtime"]["agent_profile_graceful_shutdown"] = json!("double-ctrl-c");
    write_record(&fixture);
    write_activity(&fixture, "waiting", json!([]));
    fixture
}

fn write_activity(fixture: &Fixture, phase: &str, open_tool_calls: Value) {
    let runtime = incarnation(fixture);
    let identity = Sha256::digest(format!(
        "agent-session.provider-identifier.v1\0{runtime}\0claude\0session\0resume-session-id"
    ));
    let identity: String = identity.iter().map(|byte| format!("{byte:02x}")).collect();
    fs::write(
        fixture.session.join("activity.json"),
        json!({
            "schema_version": "agent-session.activity.v1",
            "runtime_id": runtime,
            "runtime_generation": 1,
            "provider_session_id": format!("local:v1:{identity}"),
            "open_tool_calls": open_tool_calls,
            "state": {
                "schema_version": "agent-session.turn-state.v1",
                "phase": phase, "phase_changed_at": "2000-01-01T00:00:00Z",
                "revision": 2,
                "source": {"kind": "provider_hook", "confidence": "observed", "provider": "claude"}
            }
        })
        .to_string(),
    )
    .unwrap();
}

fn delete_reaping_pane(fixture: &mut Fixture, guard: &[&str]) -> nils_test_support::cmd::CmdOutput {
    let cwd = fixture._tmp.path();
    let (state, tmux) = (&fixture.state, &fixture.tmux);
    thread::scope(|scope| {
        let delete = scope.spawn(|| delete_at(cwd, state, tmux, guard));
        // Reap the pane child as tmux would, so process-group teardown can finish.
        while !delete.is_finished() && fixture.pane.0.try_wait().unwrap().is_none() {
            thread::sleep(Duration::from_millis(10));
        }
        delete.join().unwrap()
    })
}

#[test]
fn idle_delete_retires_claude_with_an_empty_composer() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    let guard = [
        "--if-idle",
        "--expected-incarnation",
        &runtime,
        "--if-unread-empty",
    ];
    let output = delete_reaping_pane(&mut fixture, &guard);
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
    assert_eq!(
        fs::read_to_string(fixture._tmp.path().join("input-events")).unwrap(),
        "paused\n"
    );
    assert!(!fixture.session.exists());
    assert_eq!(fixture.pane.0.wait().unwrap().signal(), Some(libc::SIGINT));
}

#[test]
fn idle_delete_refuses_active_or_unknown_claude() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    let guard = ["--if-idle", "--expected-incarnation", &runtime];
    let provider_state = fixture._tmp.path().join("claude-agents.json");

    // Hook activity can still say "waiting" while Claude is already busy.
    for (status, state) in [("busy", "working"), ("waiting", "permission-prompt")] {
        fs::write(
            &provider_state,
            json!([{"sessionId":"resume-session-id","status": status,
                    "waitingFor":"permission prompt"}])
            .to_string(),
        )
        .unwrap();
        let error = assert_refused(&mut fixture, &guard, "session-state-mismatch");
        assert_eq!(error["details"]["state"], state);
    }
    for output in [
        "{invalid",
        r#"[{"sessionId":"other-conversation","status":"idle"}]"#,
        r#"[{"sessionId":"resume-session-id","status":"unknown"}]"#,
    ] {
        fs::write(&provider_state, output).unwrap();
        assert_refused(&mut fixture, &guard, "session-status-unknown");
    }
    fs::write(
        &provider_state,
        r#"[{"sessionId":"resume-session-id","status":"idle"}]"#,
    )
    .unwrap();
    write_activity(&fixture, "working", json!([]));
    assert_refused(&mut fixture, &guard, "session-state-mismatch");
    fs::write(fixture.session.join("activity.json"), "{invalid").unwrap();
    assert_refused(&mut fixture, &guard, "session-status-unknown");
}

#[test]
fn idle_delete_refuses_claude_with_open_tool_or_hook_ingress() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    let guard = ["--if-idle", "--expected-incarnation", &runtime];
    write_activity(
        &fixture,
        "waiting",
        json!([{"turn_id":"fixture-turn", "call_id":"fixture-call", "input_digest":"fixture-digest"}]),
    );
    let error = assert_refused(&mut fixture, &guard, "session-state-mismatch");
    assert_eq!(error["details"]["state"], "provider-tool-pending");

    write_activity(&fixture, "waiting", json!([]));
    let ingress = fs::File::create(fixture.session.join(".activity-ingress.lock")).unwrap();
    assert_eq!(
        unsafe { libc::flock(ingress.as_raw_fd(), libc::LOCK_SH) },
        0
    );
    let error = assert_refused(&mut fixture, &guard, "session-state-mismatch");
    assert_eq!(error["details"]["state"], "provider-event-pending");
}

#[test]
fn idle_delete_preserves_claude_draft_and_checks_incarnation_and_mail() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    let guard = ["--if-idle", "--expected-incarnation", &runtime];
    unread::seed_mailbox(&fixture, &runtime);
    write_activity(&fixture, "waiting", json!([]));
    assert_refused(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", "different-runtime"],
        "session-incarnation-conflict",
    );

    let composer = fixture._tmp.path().join("composer");
    let draft = EMPTY_COMPOSER.replace("❯ ", "❯ owner draft");
    fs::write(&composer, &draft).unwrap();
    assert_refused(&mut fixture, &guard, "send-composer-not-empty");
    assert_eq!(fs::read_to_string(&composer).unwrap(), draft);
    fs::write(&composer, EMPTY_COMPOSER).unwrap();

    let sent = unread::deliver(&fixture);
    assert_eq!(sent.code, 0, "{}", sent.stdout_text());
    assert_refused(
        &mut fixture,
        &[
            "--if-idle",
            "--expected-incarnation",
            &runtime,
            "--if-unread-empty",
        ],
        "session-mailbox-not-empty",
    );
}

#[test]
fn idle_delete_rechecks_claude_after_composer_capture() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    // Change provider state deterministically during the capture command.
    // The recheck must not reuse the first successful native sample.
    let script = fs::read_to_string(&fixture.tmux).unwrap().replace(
        " capture-pane) cat ",
        r#" capture-pane) printf '%s' '[{"sessionId":"resume-session-id","status":"busy"}]' > claude-agents.json; cat "#,
    );
    fs::write(&fixture.tmux, script).unwrap();
    let error = assert_refused(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", &runtime],
        "session-state-mismatch",
    );
    assert_eq!(error["details"]["state"], "working");
}

#[test]
fn idle_delete_preserves_draft_appearing_during_final_claude_query() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    let cwd = fixture._tmp.path();
    let draft = EMPTY_COMPOSER.replace("❯ ", "❯ owner draft");
    fs::write(cwd.join("late-draft"), &draft).unwrap();
    crate::cli::write_executable(
        &cwd.join("claude"),
        r#"#!/bin/sh
[ "$*" = 'agents --json' ] || exit 42
if [ -f native-query-seen ]; then
    # Simulate input accepted just before the pause, rendered while Claude answers.
    cp late-draft composer
    printf 'final-query\n' >> input-events
fi
: > native-query-seen
exec cat claude-agents.json
"#,
    );
    let record = fixture.session.join("session.json");
    let before = fs::read(&record).unwrap();

    let output = delete_reaping_pane(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", &runtime],
    );
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "send-composer-not-empty"
    );
    let cwd = fixture._tmp.path();
    assert_eq!(fs::read(&record).unwrap(), before);
    assert!(!fixture.killed.exists());
    assert!(fixture.pane.0.try_wait().unwrap().is_none());
    assert_eq!(fs::read_to_string(cwd.join("composer")).unwrap(), draft);
    assert_eq!(
        fs::read_to_string(cwd.join("input-events")).unwrap(),
        "paused\nfinal-query\nresumed\n"
    );
    assert!(!fixture.session.join(RECEIPT).exists());
}

#[test]
fn idle_delete_refuses_when_tmux_shutdown_hook_can_reopen_input() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    fs::write(
        fixture._tmp.path().join("shutdown-hook"),
        "run-shell 'sleep 1'",
    )
    .unwrap();
    assert_refused(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", &runtime],
        "session-status-unknown",
    );
    assert!(!fixture._tmp.path().join("input-events").exists());
}

const RECEIPT: &str = "pane-input-pause.json";

fn interrupt_during_composer_check(fixture: &Fixture, interrupt: &str) {
    let script = fs::read_to_string(&fixture.tmux).unwrap().replace(
        " capture-pane) cat ",
        &format!(
            " capture-pane) [ ! -f interrupt-once ] || {{ rm interrupt-once; {interrupt}; }}; cat "
        ),
    );
    fs::write(&fixture.tmux, script).unwrap();
    fs::write(fixture._tmp.path().join("interrupt-once"), "").unwrap();
}

#[test]
fn idle_delete_restores_the_pane_a_killed_run_left_paused() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    let guard = ["--if-idle", "--expected-incarnation", &runtime];
    interrupt_during_composer_check(&fixture, "kill -9 $PPID");
    let cwd = fixture._tmp.path().to_path_buf();
    let killed = delete_at(&cwd, &fixture.state, &fixture.tmux, &guard);
    assert_ne!(killed.code, 0);
    // No destructor ran: the pause and its receipt are all that is left.
    assert_eq!(
        fs::read_to_string(cwd.join("input-events")).unwrap(),
        "paused\n"
    );
    let receipt: Value =
        serde_json::from_slice(&fs::read(fixture.session.join(RECEIPT)).unwrap()).unwrap();
    assert_eq!(receipt["pane_id"], "%91");
    assert_eq!(receipt["socket_path"], "fixture.sock");
    assert_eq!(receipt["id"], ID);
    assert_eq!(receipt["launch_id"], runtime);

    // The retry restores the recorded pane first, then retires normally.
    let output = delete_reaping_pane(&mut fixture, &guard);
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    assert_eq!(
        fs::read_to_string(cwd.join("input-events")).unwrap(),
        "paused\nresumed\npaused\n"
    );
    assert!(!fixture.session.exists());
}

#[test]
fn idle_delete_restores_the_pane_when_terminated_during_the_final_checks() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    interrupt_during_composer_check(&fixture, "kill -TERM $PPID; sleep 5");
    let record = fixture.session.join("session.json");
    let before = fs::read(&record).unwrap();
    let cwd = fixture._tmp.path().to_path_buf();
    let output = delete_at(
        &cwd,
        &fixture.state,
        &fixture.tmux,
        &["--if-idle", "--expected-incarnation", &runtime],
    );
    assert_ne!(output.code, 0);
    assert_eq!(
        fs::read_to_string(cwd.join("input-events")).unwrap(),
        "paused\nresumed\n"
    );
    assert!(!cwd.join("input-off").exists());
    assert!(!fixture.session.join(RECEIPT).exists());
    assert_eq!(fs::read(&record).unwrap(), before);
    assert!(!fixture.killed.exists());
    assert!(fixture.pane.0.try_wait().unwrap().is_none());
}

#[test]
fn idle_delete_ignores_a_receipt_for_a_different_pane_identity() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    let cwd = fixture._tmp.path().to_path_buf();
    fs::write(cwd.join("input-off"), "").unwrap();
    let receipt = json!({
        "schema_version": "agent-session.pane-input-pause.v1",
        "id": ID,
        "launch_id": runtime,
        "tmux_session": fixture.record["tmux_session"],
        "socket_path": "another-server.sock",
        "pane_id": "%91",
    });
    fs::write(fixture.session.join(RECEIPT), receipt.to_string()).unwrap();
    assert_refused(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", &runtime],
        "session-status-unknown",
    );
    assert!(!cwd.join("input-events").exists());
    assert!(cwd.join("input-off").exists());
}

#[test]
fn idle_delete_still_refuses_a_pane_disabled_without_a_receipt() {
    let mut fixture = claude_fixture();
    let runtime = incarnation(&fixture).to_owned();
    fs::write(fixture._tmp.path().join("input-off"), "").unwrap();
    assert_refused(
        &mut fixture,
        &["--if-idle", "--expected-incarnation", &runtime],
        "session-status-unknown",
    );
    assert!(!fixture._tmp.path().join("input-events").exists());
}
