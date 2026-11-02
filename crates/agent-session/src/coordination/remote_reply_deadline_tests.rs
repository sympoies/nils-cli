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
