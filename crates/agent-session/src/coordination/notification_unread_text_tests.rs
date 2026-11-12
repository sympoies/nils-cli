// Current envelope production and retired history recognition.

#[test]
fn retired_plain_reminder_producers_only_build_envelopes() {
    let name = nils_common::session_binary::NAME;
    let messages = [unread_message("recipient", "incarnation", "unread", 1000)];
    for (prompt, body) in [
        (fixed_prompt("recipient"), format!("Mailbox: {name} message inbox")),
        (coalescing::count_prompt("recipient", messages.iter()).unwrap(), format!("Mailbox: 1 unread; {name} message inbox")),
    ] {
        assert!(crate::notice::parse_notice(&prompt).unwrap().body.starts_with(&body));
        assert!(is_mailbox_reminder_prompt(&prompt));
    }
}

#[test]
fn pending_invalid_prompt_is_replaced_without_replaying_attempt_bytes() {
    let mut registry = Registry::default();
    schedule(&mut registry, "unsent", "incarnation", 100);
    schedule(&mut registry, "sent", "incarnation", 100);
    for receipt in registry.notifications.values_mut() {
        receipt.prompt = Some("Mailbox: 4 unread; agent-session message inbox".into());
        if receipt.target_session_id == "sent" {
            receipt.state = "attempt_unknown".into();
            receipt.attempted_generation = receipt.generation;
            receipt.attempted_at_epoch = 100;
            receipt.attempted_prompt = Some("retained attempted bytes".into());
            receipt.attempted_prompt_format = Some("unknown-format".into());
        }
    }
    let pending = pending_candidates(&mut registry, 200);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].target_session_id, "unsent");
    assert!(crate::notice::parse_notice(&super::super::notification_prompt(&pending[0])).is_some());
    let sent = &registry.notifications[&receipt_key("sent", "incarnation")];
    assert_eq!(sent.attempted_prompt.as_deref(), Some("retained attempted bytes"));
}

#[test]
fn total_reminder_is_bounded_by_mailbox_quota_inside_envelope() {
    crate::command_identity::with_name("agent-session", || {
        for total in [256, 1000] {
            let prompt = coalescing::unread_prompt(total, "recipient").unwrap();
            assert!(crate::notice::parse_notice(&prompt).unwrap().body.starts_with("Mailbox: 256 unread; agent-session message inbox\nRead:"));
        }
    });
}

#[test]
fn retired_writer_gate_off_hook_sends_nothing() {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    let _gate = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let mut registry = Registry::default();
    schedule(&mut registry, "target", "incarnation", 100);
    registry.messages.push(unread_message("target", "incarnation", "unread", 1000));
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext { state_dir: temp.path().into(), host: None };
    assert!(claim_hook_reminder(&context, &mut registry, "target", "incarnation", 200).is_none());
    assert_eq!(registry.notifications.values().next().unwrap().notified_generation, 0);
}

#[test]
fn retired_plain_notice_forms_are_unclassified() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../docs/specs/fixtures/history-row-classes-v1.json"
    )).unwrap();
    let mut recognized = Vec::new();
    for case in corpus["notice_recognition"].as_array().unwrap() {
        let text = case["input"].as_str().unwrap();
        if text.starts_with('※') || text.starts_with("Mailbox: ")
            || text.starts_with("Coordination mailbox") || text.starts_with("壓縮完成") {
            for provider in ["claude", "codex"] {
                if crate::provider_prompt::row_classes::notice::matches(provider, text, &serde_json::Value::Null) {
                    recognized.push(format!("{provider}: {text}"));
                }
            }
        }
    }
    assert_eq!(recognized, Vec::<String>::new());
}
