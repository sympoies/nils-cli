//! Authenticated remote ingress, recipient listing, and notification recovery.
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::super::{mailbox, notification};
use super::*;
use crate::cli::{MessageAckArgs, MessageCategory, MessageInboxArgs, MessageShowArgs};
use nils_common::cli_contract::OutputFormat;

fn inbox(context: &CliContext, categories: Vec<MessageCategory>, cursor: Option<String>) -> Value {
    mailbox::inbox(
        context,
        MessageInboxArgs {
            session: "recipient".into(),
            capability_file: Some(super::super::capability_path(
                context,
                "recipient",
                "recipient-incarnation",
            )),
            state: Some("unread".into()),
            categories,
            cursor,
            limit: Some(1),
            format: OutputFormat::Json,
        },
    )
    .expect("authenticated inbox")
}

fn show(context: &CliContext, id: &str) -> Value {
    mailbox::show(
        context,
        MessageShowArgs {
            session: "recipient".into(),
            message: id.into(),
            capability_file: Some(super::super::capability_path(
                context,
                "recipient",
                "recipient-incarnation",
            )),
            format: OutputFormat::Json,
        },
    )
    .expect("authenticated show")
}

#[test]
fn remote_unread_inbox_keeps_categories_pagination_and_incarnation_fences() {
    let (_temp, context) = tests::fixture();
    let mut ids = Vec::new();
    for category in [
        None,
        Some(MessageCategory::Handoff),
        Some(MessageCategory::Progress),
    ] {
        let mut envelope = tests::envelope();
        envelope.category = category;
        if category.is_some() {
            envelope.schema_version = ENVELOPE_V2.into();
        }
        ids.push(envelope.message_id.clone());
        receive(&context, "destination", envelope).unwrap();
    }
    let mut locked = lock_registry(&context).unwrap();
    let mut stale = locked.registry.messages[0].clone();
    stale.message_id = uuid::Uuid::new_v4().to_string();
    stale.recipient_incarnation = "previous-incarnation".into();
    locked.registry.messages.push(stale);
    locked.save().unwrap();
    drop(locked);

    let mut listed = Vec::new();
    let mut cursor = None;
    loop {
        let page = inbox(&context, Vec::new(), cursor);
        for message in page["messages"].as_array().unwrap() {
            assert_eq!(message["sender"]["machine"], "source");
            assert_eq!(message["state"], "unread");
            assert!(message.get("body").is_none());
            listed.push(message["message_id"].as_str().unwrap().to_string());
        }
        cursor = page["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    listed.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(listed, expected);
    let handoff = inbox(&context, vec![MessageCategory::Handoff], None);
    assert_eq!(handoff["messages"][0]["message_id"], ids[1]);
    assert_eq!(show(&context, &ids[1])["state"], "read");
    assert_eq!(
        inbox(&context, vec![MessageCategory::Handoff], None)["messages"],
        json!([])
    );
}

fn old_unknown_attempt(context: &CliContext) -> notification::NotificationCandidate {
    let envelope = tests::envelope();
    receive(context, "destination", envelope).unwrap();
    let mut locked = lock_registry(context).unwrap();
    let old_time = now_epoch() - 30;
    locked.registry.messages[0].created_at_epoch = old_time;
    locked.registry.messages[0].created_at = timestamp(old_time);
    let receipt = locked.registry.notifications.values_mut().next().unwrap();
    receipt.queued_at_epoch = old_time;
    locked.save().unwrap();
    drop(locked);
    let queued = notification::pending(context).unwrap().pop().unwrap();
    assert!(notification::begin_attempt(context, &queued).unwrap());
    let candidate = notification::unresolved(context).unwrap().pop().unwrap();
    assert!(notification::mark_unknown(context, &candidate, "submission-outcome-unknown").unwrap());
    candidate
}

#[test]
fn compacted_unknown_attempt_does_not_hide_a_new_remote_generation() {
    let (_temp, context) = tests::fixture();
    let old = old_unknown_attempt(&context);
    // A compacted transcript cannot establish whether the old fixed prompt was
    // accepted. That uncertainty must not strand newly received remote mail.
    let transcript = context.state_dir.join("compacted.jsonl");
    std::fs::write(&transcript, "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"sessionId\":\"provider-session\"}\n").unwrap();
    let source = crate::provider_prompt::ProviderPromptSource::for_history(
        "claude",
        "provider-session",
        transcript,
    )
    .unwrap();
    assert_eq!(
        crate::provider_prompt::prompt_observed_after(
            &source,
            &notification::fixed_prompt("recipient", old.queued_at_epoch),
            &old.attempted_at.as_deref().unwrap().parse().unwrap()
        ),
        None,
        "compaction removed the transcript evidence required for reconciliation"
    );
    let fresh = tests::envelope();
    receive(&context, "destination", fresh.clone()).unwrap();
    let candidates = notification::pending(&context).unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "a newer remote generation must remain wakeable after compaction"
    );
    assert_eq!(candidates[0].generation, old.generation + 1);
    let locked = lock_registry(&context).unwrap();
    let newest = locked
        .registry
        .messages
        .iter()
        .find(|m| m.message_id == fresh.message_id)
        .unwrap();
    assert_eq!(candidates[0].queued_at_epoch, newest.created_at_epoch);
    assert!(
        notification::fixed_prompt("recipient", candidates[0].queued_at_epoch)
            .contains(&timestamp(newest.created_at_epoch + 1))
    );
    drop(locked);
    assert!(
        !notification::mark_submitted(&context, &old).unwrap(),
        "an older uncertain receipt cannot consume fresh mail"
    );
}

#[test]
fn newer_remote_generation_survives_an_in_flight_attempt_becoming_unknown() {
    let (_temp, context) = tests::fixture();
    receive(&context, "destination", tests::envelope()).unwrap();
    let queued = notification::pending(&context).unwrap().pop().unwrap();
    assert!(notification::begin_attempt(&context, &queued).unwrap());
    let old = notification::unresolved(&context).unwrap().pop().unwrap();
    receive(&context, "destination", tests::envelope()).unwrap();
    assert!(
        notification::pending(&context).unwrap().is_empty(),
        "an active submission prevents a second dispatch"
    );
    let locked = lock_registry(&context).unwrap();
    assert!(
        notification::submission_fences_session(
            &locked.registry,
            "recipient",
            "recipient-incarnation"
        ),
        "new remote mail must not release the active session-admission fence"
    );
    drop(locked);
    assert!(notification::mark_unknown(&context, &old, "submission-outcome-unknown").unwrap());
    let candidates = notification::pending(&context).unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "a newer generation must not inherit an older unknown outcome"
    );
    assert_eq!(candidates[0].generation, old.generation + 1);
    let locked = lock_registry(&context).unwrap();
    assert!(
        !notification::submission_fences_session(
            &locked.registry,
            "recipient",
            "recipient-incarnation"
        ),
        "the older owner's recorded outcome releases its session-admission fence"
    );
}

#[test]
fn same_generation_unknown_outcome_does_not_authorize_a_duplicate_wake() {
    let (_temp, context) = tests::fixture();
    let old = old_unknown_attempt(&context);
    assert!(notification::pending(&context).unwrap().is_empty());
    assert_eq!(
        notification::unresolved(&context).unwrap()[0].generation,
        old.generation
    );
    let mut locked = lock_registry(&context).unwrap();
    assert!(
        notification::claim_hook_reminder(
            &mut locked.registry,
            "recipient",
            "recipient-incarnation",
            now_epoch()
        )
        .is_none()
    );
}

#[test]
fn drained_remote_mail_suppresses_serve_and_hook_delivery_after_compaction() {
    let (_temp, context) = tests::fixture();
    let envelope = tests::envelope();
    receive(&context, "destination", envelope.clone()).unwrap();
    let mut locked = lock_registry(&context).unwrap();
    let attempted = notification::claim_hook_reminder(
        &mut locked.registry,
        "recipient",
        "recipient-incarnation",
        now_epoch(),
    )
    .unwrap();
    locked.save().unwrap();
    drop(locked);
    // Compaction changes provider history, not the recipient incarnation.
    let transcript = context.state_dir.join("compacted.jsonl");
    std::fs::write(&transcript, "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"sessionId\":\"provider-session\"}\n").unwrap();
    let source = crate::provider_prompt::ProviderPromptSource::for_history(
        "claude",
        "provider-session",
        transcript,
    )
    .unwrap();
    assert_eq!(
        crate::provider_prompt::prompt_observed_after(
            &source,
            &notification::fixed_prompt("recipient", attempted.queued_at_epoch),
            &attempted.attempted_at.as_deref().unwrap().parse().unwrap()
        ),
        None
    );
    let fresh = tests::envelope();
    receive(&context, "destination", fresh.clone()).unwrap();
    for id in [&envelope.message_id, &fresh.message_id] {
        let shown = show(&context, id);
        let acknowledged = mailbox::ack(
            &context,
            MessageAckArgs {
                session: "recipient".into(),
                message: id.clone(),
                if_revision: shown["revision"].as_u64().unwrap(),
                idempotency_key: format!("ack-drained-{id}"),
                capability_file: Some(super::super::capability_path(
                    &context,
                    "recipient",
                    "recipient-incarnation",
                )),
                format: OutputFormat::Json,
            },
        )
        .unwrap();
        assert_eq!(acknowledged["state"], "acknowledged");
    }
    let mut locked = lock_registry(&context).unwrap();
    assert_eq!(
        notification::pending_candidates(&mut locked.registry, now_epoch()).len(),
        1,
        "the generation remains queued; suppression must depend on drained mail"
    );
    assert!(notification::deliverable_candidates(&mut locked.registry, now_epoch()).is_empty());
    assert!(
        notification::claim_hook_reminder(
            &mut locked.registry,
            "recipient",
            "recipient-incarnation",
            now_epoch()
        )
        .is_none()
    );
}

#[test]
fn migrated_queue_timestamp_is_recovered_from_newest_live_remote_mail() {
    let (_temp, context) = tests::fixture();
    receive(&context, "destination", tests::envelope()).unwrap();
    let mut locked = lock_registry(&context).unwrap();
    let newest = locked.registry.messages[0].created_at_epoch;
    // A prior receipt writer preserves its compatibility id but drops additive
    // generation and queue fields. The canonical unread record survives.
    let receipt = locked.registry.notifications.values_mut().next().unwrap();
    receipt.generation = 0;
    receipt.queued_at_epoch = 0;
    receipt.attempted_at_epoch = newest - 30;
    locked.save().unwrap();
    drop(locked);
    let pending = notification::pending(&context).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].queued_at_epoch, newest,
        "the reminder must date the newest live unread message"
    );
}

#[test]
fn migrated_carried_mail_uses_its_current_incarnation_admission_time() {
    let (_temp, context) = tests::fixture();
    receive(&context, "destination", tests::envelope()).unwrap();
    let mut locked = lock_registry(&context).unwrap();
    let carried_at = now_epoch();
    // Carry-forward preserves creation time but admits unread guidance into
    // the current incarnation at the recorded forwarding boundary.
    let message = &mut locked.registry.messages[0];
    message.sender_session_id = "local-controller".into();
    message.created_at_epoch = carried_at - 30;
    message.created_at = timestamp(carried_at - 30);
    message.forwarded_from_incarnation = Some("previous-incarnation".into());
    message.forwarded_at_epoch = Some(carried_at);
    let receipt = locked.registry.notifications.values_mut().next().unwrap();
    receipt.generation = 0;
    receipt.queued_at_epoch = 0;
    receipt.attempted_at_epoch = carried_at - 30;
    locked.save().unwrap();
    drop(locked);
    let pending = notification::pending(&context).unwrap();
    assert_eq!(
        pending[0].queued_at_epoch, carried_at,
        "a carried message must not date the new incarnation with its original creation time"
    );
}
