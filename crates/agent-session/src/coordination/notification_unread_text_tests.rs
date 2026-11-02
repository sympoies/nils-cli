// The plain reminder states one total in a readable line. The old letter
// aliases are never produced; they are only read, to rewrite an unsent receipt
// and to recognize reminders already in provider history.

const TOTAL_REMINDER: &str = "Mailbox: 4 unread; agent-session message inbox";

#[test]
fn configured_mailbox_name_reaches_reminders_and_notice_body() {
    let name = nils_common::session_binary::NAME;
    assert_eq!(
        fixed_prompt("recipient", 100),
        format!("Mailbox: {name} message inbox")
    );
    let messages = [unread_message("recipient", "incarnation", "unread", 1000)];
    let prompt = coalescing::count_prompt(messages.iter());
    assert_eq!(prompt, format!("Mailbox: 1 unread; {name} message inbox"));
    assert!(is_mailbox_reminder_prompt(&prompt));
    assert_eq!(
        crate::notice::render_summary(
            "mailbox.wake",
            &coalescing::explained_summary(messages.iter()),
            "",
        ),
        format!("Mailbox: 1 uncategorized; {name} message inbox")
    );
}

#[test]
fn configured_mailbox_name_readback_retains_default_history_and_human_rows() {
    for configured in ["session", "session-control-for-coordination"] {
        crate::command_identity::with_name(configured, || {
            for name in [configured, "agent-session"] {
                for prompt in [
                    format!("Mailbox: {name} message inbox"),
                    format!("Mailbox: 256 unread; {name} message inbox"),
                    format!("Mailbox: 2030-01-01T00:00:00Z; {name} message inbox"),
                ] {
                    assert!(is_mailbox_reminder_prompt(&prompt), "{prompt}");
                    let row = json!({"sessionId":"fixture", "type":"user", "promptSource":"typed", "message":{"role":"user", "content":prompt}});
                    assert!(
                        crate::provider_prompt::parse_history_user_prompt(
                            "claude",
                            "fixture",
                            &row.to_string()
                        )
                        .is_none()
                    );
                }
                for human in [
                    format!("Mailbox: {name} message inbox and explain it"),
                    format!("Mailbox: 257 unread; {name} message inbox"),
                    format!("Mailbox: 01 unread; {name} message inbox"),
                    format!("Mailbox: not-a-date; {name} message inbox"),
                    format!("Mailbox: 1 unread; {name} message inbox --session other"),
                ] {
                    assert!(!is_mailbox_reminder_prompt(&human), "{human}");
                }
            }
            assert!(!is_mailbox_reminder_prompt(
                "Mailbox: other-command message inbox"
            ));
        });
    }
}

#[test]
fn configured_mailbox_name_updates_pending_counts_without_rewriting_attempts() {
    let mut registry = Registry::default();
    schedule(&mut registry, "unsent", "incarnation", 100);
    schedule(&mut registry, "sent", "incarnation", 100);
    for receipt in registry.notifications.values_mut() {
        receipt.prompt = Some(TOTAL_REMINDER.to_string());
        if receipt.target_session_id == "sent" {
            receipt.state = "attempt_unknown".to_string();
            receipt.attempted_generation = receipt.generation;
            receipt.attempted_at_epoch = 100;
            receipt.attempted_prompt = Some(TOTAL_REMINDER.to_string());
            receipt.attempted_prompt_format = Some("mailbox-counts.v1".to_string());
            receipt.composer_recovery_prompt = Some(TOTAL_REMINDER.to_string());
        }
    }
    normalize_registry(&mut registry, 200);
    let pending = pending_candidates(&mut registry, 200);
    let expected = format!(
        "Mailbox: 4 unread; {} message inbox",
        nils_common::session_binary::NAME
    );
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].prompt.as_deref(), Some(expected.as_str()));
    let sent = &registry.notifications[&receipt_key("sent", "incarnation")];
    assert_eq!(sent.attempted_prompt.as_deref(), Some(TOTAL_REMINDER));
    assert_eq!(
        sent.composer_recovery_prompt.as_deref(),
        Some(TOTAL_REMINDER)
    );
    assert_eq!(
        sent.attempted_prompt_format.as_deref(),
        Some("mailbox-counts.v1")
    );
}

#[test]
fn plain_reminder_states_the_total_in_one_readable_line_with_the_notice_gate_off() {
    use crate::cli::MessageCategory;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    let expected = format!(
        "Mailbox: 4 unread; {} message inbox",
        nils_common::session_binary::NAME
    );
    let _gate = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let mut registry = Registry::default();
    schedule(&mut registry, "target", "incarnation", 100);
    let categories = [
        Some(MessageCategory::Handoff),
        Some(MessageCategory::Report),
        None,
        Some(MessageCategory::Report),
    ];
    for (index, category) in categories.into_iter().enumerate() {
        let mut message = unread_message("target", "incarnation", "unread", 1000 + index as i64);
        message.category = category;
        coalescing::admit(&mut registry, &message, 600, 100);
        registry.messages.push(message);
    }
    // The typed composer reminder.
    let receipt = registry.notifications.values().next().unwrap();
    assert_eq!(receipt.prompt.as_deref(), Some(expected.as_str()));
    // The gate-off hook fallback delivers the same bytes and records them.
    let candidate = claim_hook_reminder(&mut registry, "target", "incarnation", 200).unwrap();
    assert_eq!(candidate.prompt.as_deref(), Some(expected.as_str()));
    let receipt = registry.notifications.values().next().unwrap();
    assert_eq!(receipt.attempted_prompt.as_deref(), Some(expected.as_str()));
}

#[test]
fn total_reminder_fits_the_composer_bound_at_the_mailbox_quota() {
    crate::command_identity::with_name("agent-session", || {
        let messages: Vec<_> = (0..256)
            .map(|index| unread_message("target", "incarnation", "unread", 1000 + index))
            .collect();
        let prompt = coalescing::count_prompt(messages.iter());
        assert_eq!(prompt, "Mailbox: 256 unread; agent-session message inbox");
        assert_eq!(prompt.len(), 48);
        assert!(prompt.is_ascii() && prompt.len() <= 51);
        assert!(coalescing::is_count_prompt(&prompt));
    });
}

#[test]
fn unsent_receipts_are_rewritten_and_sent_attempts_keep_their_bytes() {
    let mut registry = Registry::default();
    schedule(&mut registry, "unsent", "incarnation", 100);
    schedule(&mut registry, "sent", "incarnation", 100);
    let old = "Mailbox: u=1";
    {
        let unsent = registry
            .notifications
            .get_mut(&receipt_key("unsent", "incarnation"))
            .unwrap();
        unsent.prompt = Some("Mailbox: h=1 r=2".to_string());
        let sent = registry
            .notifications
            .get_mut(&receipt_key("sent", "incarnation"))
            .unwrap();
        sent.state = "attempt_unknown".to_string();
        sent.attempted_generation = sent.generation;
        sent.attempted_at_epoch = 100;
        sent.attempted_prompt = Some(old.to_string());
        sent.attempted_prompt_format = Some("mailbox-counts.v1".to_string());
        sent.composer_recovery_prompt = Some(old.to_string());
        sent.prompt = Some(old.to_string());
    }
    normalize_registry(&mut registry, 200);
    let unsent = &registry.notifications[&receipt_key("unsent", "incarnation")];
    let expected = format!(
        "Mailbox: 3 unread; {} message inbox",
        nils_common::session_binary::NAME
    );
    assert_eq!(unsent.prompt.as_deref(), Some(expected.as_str()));
    let sent = &registry.notifications[&receipt_key("sent", "incarnation")];
    assert_eq!(sent.attempted_prompt.as_deref(), Some(old));
    assert_eq!(sent.composer_recovery_prompt.as_deref(), Some(old));
    assert_eq!(
        sent.attempted_prompt_format.as_deref(),
        Some("mailbox-counts.v1")
    );
    let pending = pending_candidates(&mut registry, 200);
    assert_eq!(pending.len(), 1, "a sent attempt is never queued again");
    assert_eq!(pending[0].target_session_id, "unsent");
    assert_eq!(pending[0].prompt.as_deref(), Some(expected.as_str()));
}

#[test]
fn history_still_recognizes_the_old_letter_form_as_the_one_named_exception() {
    // Provider history is external and immutable, so an old reminder already in
    // a transcript must not read as a human prompt.
    assert!(is_mailbox_reminder_prompt("Mailbox: b=1 h=2 p=3"));
    assert!(is_mailbox_reminder_prompt(TOTAL_REMINDER));
    assert!(is_mailbox_reminder_prompt(
        "Mailbox: 256 unread; agent-session message inbox"
    ));
    for human in [
        "Mailbox: 0 unread; agent-session message inbox",
        "Mailbox: 257 unread; agent-session message inbox",
        "Mailbox: 04 unread; agent-session message inbox",
        "Mailbox: -1 unread; agent-session message inbox",
        "Mailbox: 4 unread; agent-session message inbox please",
        "Mailbox: 4 unread; agent-session message inbox --session other",
        "Mailbox: 4 unread; run agent-session message inbox",
        "Mailbox: 4 unread",
        "mailbox: 4 unread; agent-session message inbox",
    ] {
        assert!(!is_mailbox_reminder_prompt(human), "{human}");
    }
}
