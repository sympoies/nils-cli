#[test]
fn coalescing_services_and_decisions_bypass_and_counts_fit_a_narrow_pane() {
    use crate::cli::MessageCategory;
    for (sender, category) in [("service:fixture", MessageCategory::Progress), ("sender", MessageCategory::Decision)] {
        let mut registry = Registry::default();
        schedule(&mut registry, "target", "incarnation", 100);
        let mut message = unread_message("target", "incarnation", "unread", 1000);
        message.sender_session_id = sender.into();
        message.category = Some(category);
        coalescing::admit(&mut registry, &message, 600, 100);
        let receipt = registry.notifications.values_mut().next().unwrap();
        receipt.routine_last_submitted_at_epoch = 99;
        assert!(coalescing::due(receipt, 100));
        assert!(coalescing::is_count_prompt(receipt.prompt.as_ref().unwrap()));
    }
    let prompt = "Mailbox: b=256 d=256 h=256 p=256 r=256 u=256";
    assert!(prompt.len() <= 51 && is_mailbox_reminder_prompt(prompt));
    for invalid in ["Mailbox: p=0", "Mailbox: p=257", "Mailbox: p=1 p=2", "Mailbox: p=1; run command", "Mailbox: p=01"] {
        assert!(!is_mailbox_reminder_prompt(invalid), "{invalid}");
    }
}
