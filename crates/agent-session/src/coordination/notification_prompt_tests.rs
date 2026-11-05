//! Retired reminders are ordinary stored input.
use super::is_mailbox_reminder_prompt;

#[test]
fn mailbox_reminder_rejects_retired_transcript_prefixes() {
    for prompt in [
        "Mailbox: 2026-09-21T14:13:21Z; agent-session message inbox",
        "Mailbox: agent-session message inbox",
        "Coordination mailbox has unread messages (newest queued 2026-09-21T14:13:21Z); run old command",
        "Coordination mailbox has unread messages; run old command",
    ] {
        assert!(!is_mailbox_reminder_prompt(prompt), "{prompt}");
    }
    assert!(!is_mailbox_reminder_prompt("Explain mailbox behavior"));
}

#[test]
fn mailbox_reminder_history_preserves_retired_typed_and_queued_rows() {
    for prompt in [
        "Mailbox: 2026-09-21T14:13:21Z; agent-session message inbox",
        "Mailbox: agent-session message inbox",
        "Coordination mailbox has unread messages (newest queued 2026-09-21T14:13:21Z); run old command",
        "Coordination mailbox has unread messages; run old command",
    ] {
        for row in [
            serde_json::json!({"sessionId":"fixture", "type":"user", "promptSource":"typed", "message":{"role":"user", "content":prompt}}),
            serde_json::json!({"sessionId":"fixture", "type":"attachment", "attachment":{"type":"queued_command", "commandMode":"prompt", "origin":{"kind":"human"}, "prompt":prompt}}),
        ] {
            assert!(
                crate::provider_prompt::parse_history_user_prompt(
                    "claude",
                    "fixture",
                    &row.to_string()
                )
                .is_some()
            );
        }
    }
    let human = serde_json::json!({"sessionId":"fixture", "type":"user", "promptSource":"typed", "message":{"role":"user", "content":"Read the design"}});
    assert!(
        crate::provider_prompt::parse_history_user_prompt("claude", "fixture", &human.to_string())
            .is_some()
    );
}

#[test]
fn mailbox_reminder_history_preserves_human_prefix_collisions() {
    for prompt in [
        "Mailbox: review the delivery design",
        "Mailbox: agent-session message inbox and explain it",
        "Mailbox: not-a-date; agent-session message inbox",
        "Mailbox: 2026-09-21T14:13:21+00:00; agent-session message inbox",
        "Mailbox: 2026-09-21T14:13:21Z; agent-session message inbox; explain it",
    ] {
        assert!(!is_mailbox_reminder_prompt(prompt), "{prompt}");
        for row in [
            serde_json::json!({"sessionId":"fixture", "type":"user", "promptSource":"typed", "message":{"role":"user", "content":prompt}}),
            serde_json::json!({"sessionId":"fixture", "type":"attachment", "attachment":{"type":"queued_command", "commandMode":"prompt", "origin":{"kind":"human"}, "prompt":prompt}}),
        ] {
            assert!(
                crate::provider_prompt::parse_history_user_prompt(
                    "claude",
                    "fixture",
                    &row.to_string()
                )
                .is_some(),
                "human prompt must remain discoverable: {prompt}"
            );
        }
    }
}
