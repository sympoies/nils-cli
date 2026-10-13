//! Delivery-boundary regressions use real locked registry and acknowledgement paths.
use super::*;
use crate::cli::{MessageAckArgs, MessageReminderArgs, MessageShowArgs};
use crate::coordination::{self, mailbox, notification};
use pretty_assertions::assert_eq;

fn authenticated_fixture() -> (tempfile::TempDir, CliContext, PathBuf) {
    let (dir, context) = fixture(
        json!({"current": receipt("queued", "current")}),
        json!([message("current", "unread", i64::MAX)]),
    );
    let token = "synthetic-current-recipient-capability-material";
    let capability = capability_path(&context, "session", "current");
    write_atomic(&capability, token.as_bytes(), SECRET_FILE_MODE).unwrap();
    let mut locked = coordination::lock_registry(&context).unwrap();
    locked
        .registry
        .brokers
        .get_mut("session")
        .unwrap()
        .capability_digest = digest_bytes(token.as_bytes());
    locked.save().unwrap();
    drop(locked);
    (dir, context, capability)
}

fn claim(context: &CliContext, capability: &Path) -> Value {
    mailbox::reminder(
        context,
        MessageReminderArgs {
            session: "session".into(),
            capability_file: Some(capability.to_path_buf()),
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    )
    .unwrap()
}

fn read_and_ack(context: &CliContext, capability: &Path) {
    let read = mailbox::show(
        context,
        MessageShowArgs {
            session: "session".into(),
            message: "synthetic-message".into(),
            capability_file: Some(capability.to_path_buf()),
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    )
    .unwrap();
    let acknowledged = mailbox::ack(
        context,
        MessageAckArgs {
            session: "session".into(),
            message: "synthetic-message".into(),
            if_revision: read["revision"].as_u64().unwrap(),
            capability_file: Some(capability.to_path_buf()),
            idempotency_key: "ack-message".into(),
            format: nils_common::cli_contract::OutputFormat::Json,
        },
    )
    .unwrap();
    assert_eq!(acknowledged["state"], "acknowledged");
}

fn delivered_then_acknowledged(use_hook: bool, compacted: bool) {
    let (_dir, context, capability) = authenticated_fixture();
    let candidate = coordination::pending_notifications(&context)
        .unwrap()
        .pop()
        .unwrap();
    if use_hook {
        assert!(claim(&context, &capability)["reminder"].is_string());
    } else {
        let mut guard =
            coordination::lock_session_quiescence(&context, "session", "current").unwrap();
        assert!(guard.begin_notification_attempt(&candidate).unwrap());
        drop(guard);
        assert!(coordination::mark_notification_submitted(&context, &candidate).unwrap());
    }
    if compacted {
        // Compaction discards provider transcript proof while the same recipient
        // and generation survive in the durable registry. Recreate the older
        // queued retry receipt without changing mailbox identity or contents.
        let mut locked = coordination::lock_registry(&context).unwrap();
        let receipt = locked.registry.notifications.values_mut().next().unwrap();
        receipt.state = "queued".into();
        receipt.notified_generation = 0;
        locked.save().unwrap();
    }
    read_and_ack(&context, &capability);
    // Exercise either boundary first so one cannot mask the other's missing save.
    if use_hook {
        assert_eq!(claim(&context, &capability)["reminder"], Value::Null);
    } else {
        let mut guard =
            coordination::lock_session_quiescence(&context, "session", "current").unwrap();
        assert!(!guard.begin_notification_attempt(&candidate).unwrap());
    }
    let locked = coordination::lock_registry_observational(&context).unwrap();
    let receipt = locked.registry.notifications.values().next().unwrap();
    assert_eq!(receipt.generation, candidate.generation);
    assert_eq!(
        receipt.notified_generation,
        if compacted { 0 } else { candidate.generation }
    );
    assert_eq!(receipt.last_reason.as_deref(), Some("mailbox-drained"));
    drop(locked);
    assert!(
        coordination::pending_notifications(&context)
            .unwrap()
            .is_empty()
    );
    assert!(
        coordination::unresolved_notifications(&context)
            .unwrap()
            .is_empty()
    );
    assert_eq!(claim(&context, &capability)["reminder"], Value::Null);
    let observed = inspect(&context);
    assert_eq!(observed["notification"]["delivery_state"], "resolved");
    assert_eq!(observed["notification"]["generation"], candidate.generation);
    assert_eq!(
        observed["notification"]["queued_at_epoch"],
        candidate.queued_at_epoch
    );
    // Fresh mail must rearm delivery rather than inherit the drained generation.
    let mut locked = coordination::lock_registry(&context).unwrap();
    locked.registry.messages[0].state = "unread".into();
    let scheduled = notification::schedule(&mut locked.registry, "session", "current", now_epoch());
    locked.save().unwrap();
    drop(locked);
    assert_eq!(scheduled.generation, candidate.generation + 1);
    assert!(claim(&context, &capability)["reminder"].is_string());
}

#[test]
fn serve_retry_resolves_acknowledged_generation() {
    delivered_then_acknowledged(false, false);
}
#[test]
fn hook_claim_resolves_acknowledged_generation() {
    delivered_then_acknowledged(true, false);
}
#[test]
fn serve_retry_resolves_acknowledged_generation_after_compaction() {
    delivered_then_acknowledged(false, true);
}
#[test]
fn hook_claim_resolves_acknowledged_generation_after_compaction() {
    delivered_then_acknowledged(true, true);
}

#[test]
fn discovery_resolves_drained_unknown_attempt_without_transcript_proof() {
    let (_dir, context, capability) = authenticated_fixture();
    let candidate = coordination::pending_notifications(&context)
        .unwrap()
        .pop()
        .unwrap();
    let mut guard = coordination::lock_session_quiescence(&context, "session", "current").unwrap();
    assert!(guard.begin_notification_attempt(&candidate).unwrap());
    drop(guard);
    assert!(
        coordination::mark_notification_unknown(&context, &candidate, "submission-outcome-unknown")
            .unwrap()
    );
    read_and_ack(&context, &capability);
    assert!(
        coordination::unresolved_notifications(&context)
            .unwrap()
            .is_empty()
    );
    let locked = coordination::lock_registry_observational(&context).unwrap();
    let receipt = locked.registry.notifications.values().next().unwrap();
    assert_eq!(receipt.notified_generation, 0);
    assert_eq!(receipt.last_reason.as_deref(), Some("mailbox-drained"));
}

#[test]
fn inspection_distinguishes_queued_submitted_and_resolved_delivery() {
    let (_dir, context, capability) = authenticated_fixture();
    assert_eq!(
        inspect(&context)["notification"]["delivery_state"],
        "queued"
    );
    assert!(claim(&context, &capability)["reminder"].is_string());
    assert_eq!(
        inspect(&context)["notification"]["delivery_state"],
        "submitted"
    );
    read_and_ack(&context, &capability);
    assert_eq!(
        inspect(&context)["notification"]["delivery_state"],
        "resolved"
    );
}

#[test]
fn drained_discovery_preserves_the_active_submission_fence_until_outcome() {
    let (_dir, context, capability) = authenticated_fixture();
    let candidate = coordination::pending_notifications(&context)
        .unwrap()
        .pop()
        .unwrap();
    let mut guard = coordination::lock_session_quiescence(&context, "session", "current").unwrap();
    assert!(guard.begin_notification_attempt(&candidate).unwrap());
    drop(guard);
    read_and_ack(&context, &capability);
    assert!(
        coordination::pending_notifications(&context)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        coordination::unresolved_notifications(&context)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(claim(&context, &capability)["reminder"], Value::Null);
    let locked = coordination::lock_registry_observational(&context).unwrap();
    assert!(notification::submission_fences_session(
        &locked.registry,
        "session",
        "current"
    ));
    drop(locked);
    assert!(coordination::mark_notification_submitted(&context, &candidate).unwrap());
    assert!(
        coordination::pending_notifications(&context)
            .unwrap()
            .is_empty()
    );
    let locked = coordination::lock_registry_observational(&context).unwrap();
    assert_eq!(
        locked
            .registry
            .notifications
            .values()
            .next()
            .unwrap()
            .last_reason
            .as_deref(),
        Some("mailbox-drained")
    );
}

#[test]
fn live_unread_for_another_incarnation_does_not_keep_the_generation_pending() {
    let (_dir, context, capability) = authenticated_fixture();
    let mut locked = coordination::lock_registry(&context).unwrap();
    let mut old = locked.registry.messages[0].clone();
    old.message_id = "older-message".into();
    old.recipient_incarnation = "older-incarnation".into();
    locked.registry.messages.push(old);
    locked.save().unwrap();
    drop(locked);
    read_and_ack(&context, &capability);
    assert!(
        coordination::pending_notifications(&context)
            .unwrap()
            .is_empty()
    );
    assert_eq!(claim(&context, &capability)["reminder"], Value::Null);
    let locked = coordination::lock_registry_observational(&context).unwrap();
    assert_eq!(
        locked
            .registry
            .notifications
            .values()
            .next()
            .unwrap()
            .notified_generation,
        0
    );
    assert_eq!(locked.registry.messages[1].state, "unread");
}
