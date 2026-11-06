use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn quota_warning_inbox_is_independent_of_page_and_state_filter() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path().join("state");
    fs::create_dir(&state).unwrap();
    super::coordination::seed_brokers(
        &state,
        &[(
            "recipient",
            "incarnation",
            "fixture-capability-material-0000000001",
        )],
    );
    let registry_path = state.join("coordination/registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let messages = (0..205).map(|index| json!({
        "schema_version": "agent-session.message.v1", "message_id": format!("message-{index}"),
        "sender_session_id": "sender", "sender_incarnation": "sender-incarnation",
        "recipient_session_id": "recipient", "recipient_incarnation": "incarnation",
        "state": "unread", "revision": 1, "reply_to": null, "reply_depth": 0,
        "created_at": "fixture", "created_at_epoch": now, "created_at_epoch_millis": now * 1000,
        "expires_at": "fixture", "expires_at_epoch": now + 3600,
        "body_bytes": 1, "body": "x",
    })).collect::<Vec<_>>();
    registry["messages"] = json!(messages);
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let capability = super::coordination::capability(&state, "recipient");
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "message",
            "inbox",
            "--session",
            "recipient",
            "--capability-file",
            &capability,
            "--state",
            "read",
            "--limit",
            "1",
            "--format",
            "json",
        ],
        &CmdOptions::new()
            .without_ambient_managed_session_env()
            .with_cwd(tmp.path()),
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let data = &output.stdout_json()["data"];
    assert_eq!(data["messages"], json!([]));
    assert_eq!(data["mailbox_quota_warning"]["status"], "near-limit");
    assert_eq!(data["mailbox_quota_warning"]["message_count"], 205);
    assert_eq!(
        data["mailbox_quota_warning"]["recovery"]["action"],
        "read_and_ack"
    );
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(registry_path).unwrap()).unwrap();
    assert_eq!(after["messages"], registry["messages"]);
}

fn quota_fixture(count: usize) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path();
    super::coordination::seed_brokers(
        state,
        &[
            (
                "recipient",
                "incarnation",
                "fixture-recipient-capability-material",
            ),
            (
                "sender",
                "sender-incarnation",
                "fixture-sender-capability-material",
            ),
        ],
    );
    let path = state.join("coordination/registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    registry["messages"] =
        json!((0..count).map(|index| json!({
        "schema_version":"agent-session.message.v1", "message_id":format!("seed-{index}"),
        "sender_session_id":"filler", "sender_incarnation":"filler-incarnation",
        "recipient_session_id":"recipient", "recipient_incarnation":"incarnation",
        "state":"read", "revision":2, "reply_to":null, "reply_depth":0,
        "created_at":"fixture", "created_at_epoch":now - 120,
        "expires_at":"fixture", "expires_at_epoch":now + 3600,
        "body_bytes":1, "body":"x"
    })).collect::<Vec<_>>());
    fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
    tmp
}

fn quota_command(state: &std::path::Path, args: &[&str]) -> nils_test_support::cmd::CmdOutput {
    let mut all = vec!["--state-dir", state.to_str().unwrap()];
    all.extend_from_slice(args);
    run_resolved(
        "agent-session",
        &all,
        &CmdOptions::new()
            .without_ambient_managed_session_env()
            .with_cwd(state)
            .with_env("AGENT_SESSION_NOTICE_WRITERS", "1"),
    )
}

#[test]
fn quota_rejected_send_records_body_free_recipient_event_and_notice() {
    let tmp = quota_fixture(256);
    let state = tmp.path();
    let body = state.join("rejected.txt");
    fs::write(&body, "REJECTED_BODY_CANARY").unwrap();
    let cap = super::coordination::capability(state, "sender");
    let sent = quota_command(
        state,
        &[
            "message",
            "send",
            "--from",
            "sender",
            "--to",
            "recipient",
            "--capability-file",
            &cap,
            "--body-file",
            body.to_str().unwrap(),
            "--category",
            "handoff",
            "--idempotency-key",
            "rejected",
            "--format",
            "json",
        ],
    );
    assert_eq!(sent.stdout_json()["error"]["code"], "quota-exceeded");
    let audit = quota_command(state, &["message", "audit", "--format", "json"]);
    assert_eq!(audit.code, 0, "{}", audit.stdout_text());
    let events = audit.stdout_json()["data"]["quota_rejections"].clone();
    assert!(
        events.is_array(),
        "recipient rejection event is missing: {events}"
    );
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["recipient"]["session_id"], "recipient");
    assert_eq!(events[0]["sender"]["session_id"], "sender");
    assert_eq!(events[0]["category"], "handoff");
    assert!(events[0]["occurred_at"].is_string());
    assert!(!audit.stdout_text().contains("REJECTED_BODY_CANARY"));
    let stored = fs::read_to_string(state.join("coordination/registry.json")).unwrap();
    assert!(!stored.contains("REJECTED_BODY_CANARY"));
    let registry: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(registry["messages"].as_array().unwrap().len(), 256);
    let cap = super::coordination::capability(state, "recipient");
    let reminder = quota_command(
        state,
        &[
            "message",
            "reminder",
            "--session",
            "recipient",
            "--capability-file",
            &cap,
            "--format",
            "json",
        ],
    );
    assert_eq!(reminder.code, 0, "{}", reminder.stdout_text());
    assert!(
        reminder.stdout_json()["data"]["reminder"]
            .as_str()
            .unwrap()
            .contains("mailbox.alert")
    );
}

fn quota_send(state: &std::path::Path, key: &str) -> serde_json::Value {
    let body = state.join("body.txt");
    fs::write(&body, "x").unwrap();
    let cap = super::coordination::capability(state, "sender");
    let result = quota_command(
        state,
        &[
            "message",
            "send",
            "--from",
            "sender",
            "--to",
            "recipient",
            "--capability-file",
            &cap,
            "--body-file",
            body.to_str().unwrap(),
            "--category",
            "progress",
            "--idempotency-key",
            &format!("send-{key}"),
            "--format",
            "json",
        ],
    );
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    result.stdout_json()["data"].clone()
}

fn quota_reminder(state: &std::path::Path) -> serde_json::Value {
    let cap = super::coordination::capability(state, "recipient");
    let result = quota_command(
        state,
        &[
            "message",
            "reminder",
            "--session",
            "recipient",
            "--capability-file",
            &cap,
            "--format",
            "json",
        ],
    );
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    result.stdout_json()["data"]["reminder"].clone()
}

fn quota_ack(state: &std::path::Path, id: &str) {
    let cap = super::coordination::capability(state, "recipient");
    let result = quota_command(
        state,
        &[
            "message",
            "ack",
            "--session",
            "recipient",
            "--message",
            id,
            "--if-revision",
            "2",
            "--idempotency-key",
            &format!("acknowledge-{id}"),
            "--capability-file",
            &cap,
            "--format",
            "json",
        ],
    );
    assert_eq!(result.code, 0, "{}", result.stdout_text());
}

#[test]
fn quota_alert_is_once_per_crossing_and_rearms_only_below_threshold() {
    let tmp = quota_fixture(204);
    let state = tmp.path();
    let sent = quota_send(state, "cross");
    let registry: serde_json::Value =
        serde_json::from_slice(&fs::read(state.join("coordination/registry.json")).unwrap())
            .unwrap();
    let receipt = registry["notifications"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(sent["notification"]["generation"], receipt["generation"]);
    let first = quota_reminder(state);
    assert!(first.as_str().unwrap().contains("mailbox.alert"));
    assert!(quota_reminder(state).is_null());
    quota_send(state, "above");
    assert!(
        !quota_reminder(state)
            .as_str()
            .unwrap()
            .contains("mailbox.alert")
    );
    quota_ack(state, "seed-0"); // 205: still at the threshold.
    quota_send(state, "still-above");
    assert!(
        !quota_reminder(state)
            .as_str()
            .unwrap()
            .contains("mailbox.alert")
    );
    quota_ack(state, "seed-1");
    quota_ack(state, "seed-2"); // 204: re-arm.
    quota_send(state, "recross");
    assert_eq!(quota_reminder(state), first);
}

#[test]
fn quota_alert_uses_destination_policy_percentage() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = quota_fixture(127);
    let state = tmp.path();
    let path = state.join("coordination/notification-policy.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "schema_version":"agent-session.notification-policy.v1",
            "recipients":[{"session_id":"recipient", "suppress_categories":["progress"],
                "routine_interval_seconds":600, "mailbox_alert_percent":50}]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    quota_send(state, "configured-cross");
    assert!(
        quota_reminder(state)
            .as_str()
            .unwrap()
            .contains("mailbox.alert")
    );
}

#[test]
fn quota_alert_also_crosses_the_byte_limit_fraction() {
    let tmp = quota_fixture(1);
    let state = tmp.path();
    let path = state.join("coordination/registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    registry["messages"][0]["body_bytes"] = json!((4_usize * 1024 * 1024 * 80).div_ceil(100) - 1);
    fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
    quota_send(state, "byte-crossing");
    assert!(
        quota_reminder(state)
            .as_str()
            .unwrap()
            .contains("mailbox.alert")
    );
}
