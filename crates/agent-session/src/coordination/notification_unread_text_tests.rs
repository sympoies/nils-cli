// The plain reminder states one total in a readable line. The old letter
// aliases are never produced; they are only read, to rewrite an unsent receipt
// and to recognize reminders already in provider history.

const TOTAL_REMINDER: &str = "Mailbox: 4 unread; agent-session message inbox";

#[test]
fn plain_reminder_states_the_total_in_one_readable_line_with_the_notice_gate_off() {
    use crate::cli::MessageCategory;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
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
    assert_eq!(receipt.prompt.as_deref(), Some(TOTAL_REMINDER));
    // The gate-off hook fallback delivers the same bytes and records them.
    let candidate = claim_hook_reminder(&mut registry, "target", "incarnation", 200).unwrap();
    assert_eq!(candidate.prompt.as_deref(), Some(TOTAL_REMINDER));
    let receipt = registry.notifications.values().next().unwrap();
    assert_eq!(receipt.attempted_prompt.as_deref(), Some(TOTAL_REMINDER));
}

#[test]
fn total_reminder_fits_the_composer_bound_at_the_mailbox_quota() {
    let messages: Vec<_> = (0..256)
        .map(|index| unread_message("target", "incarnation", "unread", 1000 + index))
        .collect();
    let prompt = coalescing::count_prompt(messages.iter());
    assert_eq!(prompt, "Mailbox: 256 unread; agent-session message inbox");
    assert_eq!(prompt.len(), 48);
    assert!(prompt.is_ascii() && prompt.len() <= 51);
    assert!(coalescing::is_count_prompt(&prompt));
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
    assert_eq!(
        unsent.prompt.as_deref(),
        Some("Mailbox: 3 unread; agent-session message inbox")
    );
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
    assert_eq!(
        pending[0].prompt.as_deref(),
        Some("Mailbox: 3 unread; agent-session message inbox")
    );
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
