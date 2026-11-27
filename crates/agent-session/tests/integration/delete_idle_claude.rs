use super::{Fixture, ID, assert_refused, delete_at, incarnation, unread, write_record};
use pretty_assertions::assert_eq;
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
    let cwd = fixture._tmp.path();
    let (state, tmux) = (&fixture.state, &fixture.tmux);
    let output = thread::scope(|scope| {
        let delete = scope.spawn(|| delete_at(cwd, state, tmux, &guard));
        // Reap the pane child as tmux would, so process-group teardown can finish.
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
