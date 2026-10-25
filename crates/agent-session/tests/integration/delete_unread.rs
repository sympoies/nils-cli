use super::*;
use crate::coordination::{capability, load_coordination_registry, seed_brokers};
use pretty_assertions::{assert_eq, assert_ne};

fn seed_mailbox(fixture: &Fixture, incarnation: &str) {
    let record = fs::read(fixture.session.join("session.json")).unwrap();
    seed_brokers(
        &fixture.state,
        &[
            (
                "sender",
                "sender-incarnation",
                "fixture-sender-capability-material",
            ),
            (
                "guarded-delete",
                incarnation,
                "fixture-recipient-capability-material",
            ),
        ],
    );
    fs::write(fixture.session.join("session.json"), record).unwrap();
}

fn deliver(fixture: &Fixture) -> nils_test_support::cmd::CmdOutput {
    let body = fixture._tmp.path().join("body.txt");
    fs::write(&body, "a pending coordination request").unwrap();
    run(
        fixture._tmp.path(),
        &[
            "--state-dir",
            fixture.state.to_str().unwrap(),
            "message",
            "send",
            "--from",
            "sender",
            "--to",
            "guarded-delete",
            "--body-file",
            body.to_str().unwrap(),
            "--capability-file",
            &capability(&fixture.state, "sender"),
            "--idempotency-key",
            "delete-race-message",
            "--format",
            "json",
        ],
        &[],
    )
}

#[test]
fn conditional_delete_preserves_mail_delivered_after_empty_list_snapshot() {
    let mut fixture = Fixture::new();
    let incarnation = fixture.mark_runtime_stopped();
    seed_mailbox(&fixture, &incarnation);
    let listed = run(
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
    assert_eq!(listed.code, 0, "{}", listed.stdout_text());
    let snapshot = listed.stdout_json();
    let session = snapshot["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "guarded-delete")
        .unwrap();
    assert_eq!(session["unread_message_count"], 0);

    // Actual message admission runs between the caller's snapshot and deletion.
    let sent = deliver(&fixture);
    assert_eq!(sent.code, 0, "{}", sent.stdout_text());
    let message_id = sent.stdout_json()["data"]["message_id"].clone();
    let output = delete_guarded(&fixture, &incarnation);
    assert_eq!(
        output.code,
        65,
        "newly unread mail must prevent deletion: {}",
        output.stdout_text()
    );
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "session-mailbox-not-empty"
    );
    assert_eq!(
        output.stdout_json()["error"]["details"]["unread_message_count"],
        1
    );
    let details = output.stdout_json()["error"]["details"].clone();
    assert_eq!(details["predicate"], "unread-empty");
    assert_eq!(details["retryable"], false);
    assert_eq!(details["next_action"], "inspect_unread_messages");
    assert_eq!(details["recovery"]["kind"], "review_mailbox");
    assert!(
        !output
            .stdout_text()
            .contains("a pending coordination request")
    );
    assert!(
        !output
            .stdout_text()
            .contains(fixture.state.to_str().unwrap())
    );
    assert!(!output.stdout_text().contains("guarded-delete"));
    assert!(fixture.session.exists());
    assert!(!fixture.killed.exists());
    let registry = load_coordination_registry(&fixture.state);
    let message = registry["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["message_id"] == message_id)
        .unwrap();
    assert_eq!(message["state"], "unread");
    assert_eq!(message["body"], "a pending coordination request");
}

fn delete_guarded(fixture: &Fixture, incarnation: &str) -> nils_test_support::cmd::CmdOutput {
    run(
        fixture._tmp.path(),
        &[
            "--state-dir",
            fixture.state.to_str().unwrap(),
            "delete",
            "guarded-delete",
            "--if-stopped",
            "--expected-incarnation",
            incarnation,
            "--if-unread-empty",
            "--tmux-bin",
            fixture.tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    )
}

#[test]
fn unread_guard_allows_empty_stopped_mailbox_and_keeps_runtime_guards() {
    let mut fixture = Fixture::new();
    let incarnation = fixture.mark_runtime_stopped();
    seed_mailbox(&fixture, &incarnation);
    for (expected, status, code) in [
        ("replacement", "stopped", "session-incarnation-conflict"),
        (incarnation.as_str(), "running", "session-state-mismatch"),
        (incarnation.as_str(), "unknown", "session-status-unknown"),
    ] {
        fs::write(&fixture.name_status, status).unwrap();
        let output = delete_guarded(&fixture, expected);
        assert_ne!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(output.stdout_json()["error"]["code"], code);
        assert!(fixture.session.exists());
    }
    let child_path = fixture.state.join("sessions/sender/session.json");
    let mut child: Value = serde_json::from_slice(&fs::read(&child_path).unwrap()).unwrap();
    let original_child = child.clone();
    let parent = json!({"machine": "fixture-machine", "session_id": "guarded-delete",
        "session_created_at": fixture.record["created_at"]});
    child["lineage"] = json!({
        "schema_version": "agent-session.session-lineage.v1", "parent": parent,
        "root": parent, "depth": 1, "starter": {"kind": "session", "via": "cli"}
    });
    fs::write(&child_path, serde_json::to_vec(&child).unwrap()).unwrap();
    let output = delete_guarded(&fixture, &incarnation);
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "session-has-live-children"
    );
    assert!(fixture.session.exists());
    fs::write(&child_path, serde_json::to_vec(&original_child).unwrap()).unwrap();
    fs::write(&fixture.name_status, "stopped").unwrap();
    let output = delete_guarded(&fixture, &incarnation);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["deleted"], true);
    assert_eq!(output.stdout_json()["data"]["killed"], false);
    assert!(!fixture.session.exists());
    assert!(!fixture.killed.exists());
}

#[test]
fn unread_guard_refuses_unprovable_mailbox_without_removing_state() {
    let mut fixture = Fixture::new();
    let incarnation = fixture.mark_runtime_stopped();
    seed_mailbox(&fixture, &incarnation);
    let registry = fixture.state.join("coordination/registry.json");
    fs::write(&registry, "{invalid").unwrap();
    let output = delete_guarded(&fixture, &incarnation);
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "session-mailbox-unknown"
    );
    assert_eq!(
        output.stdout_json()["error"]["details"]["predicate"],
        "unread-empty"
    );
    assert_eq!(output.code, 1);
    assert_eq!(output.stdout_json()["error"]["details"]["retryable"], true);
    assert_eq!(
        output.stdout_json()["error"]["details"]["next_action"],
        "inspect_mailbox_store_then_retry"
    );
    assert_eq!(
        output.stdout_json()["error"]["details"]["recovery"]["kind"],
        "retry_conditional_delete"
    );
    assert!(
        !output
            .stdout_text()
            .contains(fixture.state.to_str().unwrap())
    );
    assert!(!output.stdout_text().contains("guarded-delete"));
    assert_eq!(fs::read_to_string(registry).unwrap(), "{invalid");
    assert!(fixture.session.exists());
    assert!(!fixture.killed.exists());
}

#[test]
fn unread_guard_does_not_block_read_mail_or_change_stopped_only_delete() {
    for guarded in [true, false] {
        let mut fixture = Fixture::new();
        let incarnation = fixture.mark_runtime_stopped();
        seed_mailbox(&fixture, &incarnation);
        let sent = deliver(&fixture);
        assert_eq!(sent.code, 0, "{}", sent.stdout_text());
        if guarded {
            let shown = run(
                fixture._tmp.path(),
                &[
                    "--state-dir",
                    fixture.state.to_str().unwrap(),
                    "message",
                    "show",
                    "--session",
                    "guarded-delete",
                    "--message",
                    sent.stdout_json()["data"]["message_id"].as_str().unwrap(),
                    "--capability-file",
                    &capability(&fixture.state, "guarded-delete"),
                    "--format",
                    "json",
                ],
                &[],
            );
            assert_eq!(shown.code, 0, "{}", shown.stdout_text());
        }
        let output = if guarded {
            delete_guarded(&fixture, &incarnation)
        } else {
            fixture.delete_if_stopped(&incarnation)
        };
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert!(!fixture.session.exists());
    }
}

#[test]
fn unread_guard_fences_concurrent_real_delivery_through_removal() {
    let mut fixture = Fixture::new();
    let incarnation = fixture.mark_runtime_stopped();
    seed_mailbox(&fixture, &incarnation);
    fs::write(&fixture.name_status, "stopped-wait").unwrap();
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let deleting = command
        .args([
            "--state-dir",
            fixture.state.to_str().unwrap(),
            "delete",
            "guarded-delete",
            "--if-stopped",
            "--expected-incarnation",
            &incarnation,
            "--if-unread-empty",
            "--tmux-bin",
            fixture.tmux.to_str().unwrap(),
            "--format",
            "json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.status_entered.exists() {
        assert!(
            Instant::now() < deadline,
            "delete did not reach stopped probe"
        );
        thread::sleep(Duration::from_millis(5));
    }
    // The delete is paused inside the lifecycle critical section. A real send
    // must wait for that fence rather than persist a message behind the check.
    let body = fixture._tmp.path().join("body.txt");
    fs::write(&body, "a concurrent coordination request").unwrap();
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let mut sending = command
        .args([
            "--state-dir",
            fixture.state.to_str().unwrap(),
            "message",
            "send",
            "--from",
            "sender",
            "--to",
            "guarded-delete",
            "--body-file",
            body.to_str().unwrap(),
            "--capability-file",
            &capability(&fixture.state, "sender"),
            "--idempotency-key",
            "concurrent-delete",
            "--format",
            "json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    assert!(
        sending.try_wait().unwrap().is_none(),
        "delivery escaped lifecycle fence"
    );
    assert!(
        load_coordination_registry(&fixture.state)["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    fs::write(&fixture.status_release, "release").unwrap();
    let deleted = deleting.wait_with_output().unwrap();
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stdout)
    );
    let sent = sending.wait_with_output().unwrap();
    assert!(!sent.status.success());
    let response: Value = serde_json::from_slice(&sent.stdout).unwrap();
    assert_eq!(response["error"]["code"], "message-not-found");
    assert!(!fixture.session.exists());
    assert!(
        load_coordination_registry(&fixture.state)["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
