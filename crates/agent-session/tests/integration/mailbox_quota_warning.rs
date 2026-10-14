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
