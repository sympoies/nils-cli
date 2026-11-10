//! Destination-owned routine windows; urgency follows authenticated metadata.
use super::{NotificationReceipt, Registry, receipt_key};
use crate::cli::MessageCategory;
use crate::coordination::mailbox::StoredMessage;
use std::collections::BTreeMap;

pub(super) fn due(receipt: &NotificationReceipt, now: i64) -> bool {
    let last = receipt
        .routine_last_submitted_at_epoch
        .max(receipt.attempted_at_epoch);
    receipt.routine_interval_seconds == 0
        || receipt.urgent_generation > receipt.notified_generation
        || last == 0
        || now.saturating_sub(last) >= receipt.routine_interval_seconds
}

pub(in crate::coordination) fn admit(
    registry: &mut Registry,
    message: &StoredMessage,
    interval: i64,
    now: i64,
) {
    let urgent = matches!(
        message.category,
        Some(MessageCategory::Blocker | MessageCategory::Decision)
    ) || crate::coordination::service::is_service_sender(&message.sender_session_id);
    if interval == 0 {
        if let Some(receipt) = registry.notifications.get_mut(&receipt_key(
            &message.recipient_session_id,
            &message.recipient_incarnation,
        )) {
            receipt.routine_interval_seconds = 0;
            receipt.prompt = None;
        }
        return;
    }
    let prompt = count_prompt(
        &message.recipient_session_id,
        registry
            .messages
            .iter()
            .filter(|item| {
                item.recipient_session_id == message.recipient_session_id
                    && item.recipient_incarnation == message.recipient_incarnation
                    && item.state == "unread"
                    && !item.notification_suppressed
                    && item.expires_at_epoch > now
            })
            .chain(std::iter::once(message)),
    );
    if let Some(receipt) = registry.notifications.get_mut(&receipt_key(
        &message.recipient_session_id,
        &message.recipient_incarnation,
    )) {
        receipt.routine_interval_seconds = interval;
        if urgent {
            receipt.urgent_generation = receipt.generation;
        }
        receipt.prompt = prompt;
    }
}

/// The mailbox quota bounds the total carried by the reminder.
const MAX_TOTAL: usize = 256;

/// Bounded unread totals are always carried in a notice envelope.
pub(super) fn unread_prompt(total: usize, session: &str) -> Option<String> {
    super::notice::encode_summary(
        &format!(
            "{} unread; {} message inbox",
            total.clamp(1, MAX_TOTAL),
            crate::executable()
        ),
        session,
    )
}

fn category_counts<'a>(
    messages: impl Iterator<Item = &'a StoredMessage>,
) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for item in messages {
        let category = match item.category.unwrap_or_default() {
            MessageCategory::Progress => "progress",
            MessageCategory::Handoff => "handoff",
            MessageCategory::Report => "report",
            MessageCategory::Blocker => "blocker",
            MessageCategory::Decision => "decision",
            MessageCategory::Uncategorized => "uncategorized",
        };
        *counts.entry(category).or_default() += 1;
    }
    counts
}

/// Render only unread metadata selected by the owning mailbox transaction.
///
/// One bounded total inside the current notice envelope.
pub(super) fn count_prompt<'a>(
    session: &str,
    messages: impl Iterator<Item = &'a StoredMessage>,
) -> Option<String> {
    unread_prompt(messages.count(), session)
}

/// The counts spelled out by category, for notice delivery. A notice envelope is
/// multiline and carries the category summary without a composer width bound.
pub(super) fn explained_summary<'a>(messages: impl Iterator<Item = &'a StoredMessage>) -> String {
    format!(
        "{}; {} message inbox",
        category_counts(messages)
            .iter()
            .map(|(name, count)| format!("{count} {name}"))
            .collect::<Vec<_>>()
            .join(", "),
        crate::executable()
    )
}
