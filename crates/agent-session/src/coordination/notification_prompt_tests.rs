//! One-row reminders and compatibility with prior transcript prefixes.
use super::{fixed_prompt, is_mailbox_reminder_prompt};

#[test]
fn mailbox_reminder_fits_one_row_for_worst_case_date_and_session() {
    let session = "s".repeat(256);
    for queued_at in [
        0,
        1_790_000_000,
        jiff::Timestamp::MAX.as_second() - 1,
        i64::MAX,
    ] {
        let prompt = fixed_prompt(&session, queued_at);
        assert!(
            prompt.is_ascii(),
            "visible width must not depend on Unicode"
        );
        assert!(prompt.len() <= 60, "{} columns: {prompt}", prompt.len());
        assert!(!prompt.contains(['\n', '\r']));
        assert!(prompt.ends_with("agent-session message inbox"));
        assert!(!prompt.contains(&session));
    }
}

#[test]
fn mailbox_reminder_recognizes_new_and_legacy_transcript_prefixes() {
    for prompt in [
        "Mailbox: 2026-09-21T14:13:21Z; agent-session message inbox",
        "Mailbox: agent-session message inbox",
        "Coordination mailbox has unread messages (newest queued 2026-09-21T14:13:21Z); run old command",
        "Coordination mailbox has unread messages; run old command",
    ] {
        assert!(is_mailbox_reminder_prompt(prompt), "{prompt}");
    }
    assert!(!is_mailbox_reminder_prompt("Explain mailbox behavior"));
}

#[test]
fn mailbox_reminder_history_hides_new_and_legacy_typed_and_queued_rows() {
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
                .is_none()
            );
        }
    }
    let human = serde_json::json!({"sessionId":"fixture", "type":"user", "promptSource":"typed", "message":{"role":"user", "content":"Read the design"}});
    assert!(
        crate::provider_prompt::parse_history_user_prompt("claude", "fixture", &human.to_string())
            .is_some()
    );
}
