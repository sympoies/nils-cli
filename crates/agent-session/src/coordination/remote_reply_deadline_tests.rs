use super::*;
use pretty_assertions::assert_eq;

#[test]
fn reply_deadline_ingress_preserves_source_time_and_refuses_out_of_range_deadlines() {
    for seconds in [0, -1, super::super::mailbox::MAX_EXPIRY_SECS + 1] {
        let (_temp, context) = tests::fixture();
        let mut message = tests::envelope();
        message.reply_by_epoch = Some(message.created_at_epoch + seconds);
        assert_eq!(
            receive(&context, "destination", message)
                .unwrap_err()
                .code(),
            "remote-message-invalid"
        );
        assert!(
            lock_registry(&context)
                .unwrap()
                .registry
                .messages
                .is_empty()
        );
    }
    let (_temp, context) = tests::fixture();
    let mut message = tests::envelope();
    message.created_at_epoch -= 20;
    message.reply_by_epoch = Some(message.created_at_epoch + 1);
    let expected = message.reply_by_epoch;
    receive(&context, "destination", message).unwrap();
    let locked = lock_registry(&context).unwrap();
    let stored = &locked.registry.messages[0];
    assert_eq!(stored.reply_by_epoch, expected);
    assert_eq!(
        super::super::mailbox::message_projection(stored)["reply_by"],
        timestamp(expected.unwrap())
    );
}

#[test]
fn reply_deadline_remote_outbox_acceptance_and_replay_clear_original_obligation() {
    let (_temp, context) = tests::fixture();
    let mut parent = tests::envelope();
    parent.reply_by_epoch = Some(parent.created_at_epoch + 1);
    let parent_id = parent.message_id.clone();
    let destination = parent.from.clone();
    receive(&context, "destination", parent).unwrap();
    let capability = std::fs::read_to_string(super::super::capability_path(
        &context,
        "recipient",
        "recipient-incarnation",
    ))
    .unwrap();
    let args = Submit {
        to_machine: destination.machine,
        to_session: destination.session_id,
        body: "bounded reply".into(),
        idempotency_key: "deadline-reply".into(),
        reply_to: Some(parent_id),
        reply_by: None,
        expires_in: None,
        reply_revision: Some(1),
        expected_recipient_incarnation: None,
        category: None,
        forward: None,
    };
    let outcome = submit(
        &context,
        &tests::config(),
        "recipient",
        &capability,
        args.clone(),
    )
    .unwrap();
    assert_eq!(outcome["state"], "queued");
    let mut locked = lock_registry(&context).unwrap();
    assert!(locked.registry.messages[0].reply_sent_at_epoch.is_some());
    // A retry reconciles a persisted outbox when its subsequent registry write was interrupted.
    locked.registry.messages[0].reply_sent_at_epoch = None;
    locked.save().unwrap();
    drop(locked);
    assert_eq!(
        submit(&context, &tests::config(), "recipient", &capability, args).unwrap(),
        outcome
    );
    assert!(
        lock_registry(&context).unwrap().registry.messages[0]
            .reply_sent_at_epoch
            .is_some()
    );
}

#[test]
fn accepted_reply_crash_gap_is_reconciled_without_resubmission() {
    let (_temp, context) = tests::fixture();
    let mut parent = tests::envelope();
    parent.created_at_epoch -= 2;
    parent.reply_by_epoch = Some(parent.created_at_epoch + 1);
    let parent_id = parent.message_id.clone();
    let destination = parent.from.clone();
    receive(&context, "destination", parent).unwrap();
    let observe = |context: &CliContext| {
        let page = super::super::audit::snapshot(
            context,
            "destination",
            super::super::audit::Query {
                include_healthy: true,
                ..Default::default()
            },
        )
        .unwrap();
        page["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["message_id"] == parent_id)
            .unwrap()
            .clone()
    };
    assert!(
        observe(&context)["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
    let capability = std::fs::read_to_string(super::super::capability_path(
        &context,
        "recipient",
        "recipient-incarnation",
    ))
    .unwrap();
    submit(
        &context,
        &tests::config(),
        "recipient",
        &capability,
        Submit {
            to_machine: destination.machine,
            to_session: destination.session_id,
            body: "accepted reply before interruption".into(),
            idempotency_key: "interrupted-deadline-reply".into(),
            reply_to: Some(parent_id.clone()),
            reply_by: None,
            expires_in: None,
            reply_revision: Some(1),
            expected_recipient_incarnation: None,
            category: None,
            forward: None,
        },
    )
    .unwrap();
    // Model the reachable post-journal/pre-registry crash boundary, then a
    // queued backoff so restart recovery cannot depend on a network attempt.
    {
        let mut journal = lock_journal(&context).unwrap();
        let reply = &mut journal.registry.remote_outbox[0];
        reply.attempts = 1;
        reply.last_attempt_at_epoch = Some(now_epoch());
        reply.next_attempt_epoch = now_epoch() + 60;
        reply.reason = Some("remote-messaging-unavailable".into());
        journal.save().unwrap();
    }
    {
        let mut registry = lock_registry(&context).unwrap();
        registry.registry.messages[0].reply_sent_at_epoch = None;
        registry.save().unwrap();
    }
    let restarted = CliContext {
        state_dir: context.state_dir.clone(),
        host: None,
    };
    let registry_path = context.state_dir.join("coordination/registry.json");
    let journal_path = context
        .state_dir
        .join("coordination/federation-journal.json");
    let before_registry = std::fs::read(&registry_path).unwrap();
    let before_journal = std::fs::read(&journal_path).unwrap();
    assert!(
        !observe(&restarted)["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply")),
        "accepted reply must not remain overdue after restart without resubmission"
    );
    assert_eq!(
        std::fs::read(&registry_path).unwrap(),
        before_registry,
        "audit stays read-only"
    );
    assert_eq!(
        std::fs::read(&journal_path).unwrap(),
        before_journal,
        "audit stays read-only"
    );
    drain(&restarted, &tests::config()).unwrap();
    assert!(
        lock_registry(&restarted).unwrap().registry.messages[0]
            .reply_sent_at_epoch
            .is_some()
    );
    {
        let mut journal = lock_journal(&restarted).unwrap();
        journal.registry.remote_outbox[0].state = "delivery-unknown".into();
        journal.registry.remote_outbox[0].reason = Some("delivery-unknown".into());
        journal.save().unwrap();
        assert!(journal.registry.remote_outbox.is_empty());
        assert_eq!(journal.registry.retained.len(), 1);
    }
    assert!(
        !observe(&restarted)["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
}
