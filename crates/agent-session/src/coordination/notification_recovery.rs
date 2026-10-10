//! Recover pending generations from canonical live mailbox metadata.
use super::{NotificationReceipt, REASON_PENDING};
use crate::coordination::Registry;

/// An unknown older submission cannot acknowledge a newer generation. Keep
/// same-generation uncertainty parked, and keep active attempts fenced until
/// their owner records an outcome.
pub(super) fn resume_new_generation(receipt: &mut NotificationReceipt, now: i64) {
    if receipt.state == "attempt_unknown"
        && receipt.attempted_generation > 0
        && receipt.generation > receipt.attempted_generation
        && receipt.generation > receipt.notified_generation
    {
        receipt.state = "queued".into();
        receipt.next_attempt_at_epoch = now;
        receipt.updated_at_epoch = now;
        receipt.last_reason = Some(REASON_PENDING.into());
    }
}

/// Receipt migrations and older writers can lose additive queue timestamps.
/// Date future deliveries from the newest unread canonical record; never
/// rewrite the timestamp retained for reconciliation of an existing attempt.
pub(super) fn refresh(registry: &mut Registry, now: i64) {
    let mut newest_by_recipient = std::collections::BTreeMap::new();
    for message in registry
        .messages
        .iter()
        .filter(|message| message.state == "unread" && message.expires_at_epoch > now)
    {
        let queued_at = message
            .forwarded_at_epoch
            .unwrap_or(message.created_at_epoch);
        let newest = newest_by_recipient
            .entry((
                message.recipient_session_id.as_str(),
                message.recipient_incarnation.as_str(),
            ))
            .or_insert(queued_at);
        *newest = (*newest).max(queued_at);
    }
    for receipt in registry.notifications.values_mut() {
        resume_new_generation(receipt, now);
        if receipt.state != "queued" {
            continue;
        }
        let newest = newest_by_recipient.get(&(
            receipt.target_session_id.as_str(),
            receipt.target_incarnation.as_str(),
        ));
        if let Some(newest) = newest {
            receipt.queued_at_epoch = receipt.queued_at_epoch.max(*newest);
        }
    }
}
