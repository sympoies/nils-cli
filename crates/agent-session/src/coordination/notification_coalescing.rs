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
        receipt.prompt = Some(prompt);
    }
}

/// The mailbox quota bounds the total, so the reminder fits one input row.
const MAX_TOTAL: usize = 256;

/// The plain reminder: one total and the inbox action, in a readable line.
pub(super) fn unread_prompt(total: usize) -> String {
    format!(
        "Mailbox: {} unread; {} message inbox",
        total.clamp(1, MAX_TOTAL),
        crate::executable()
    )
}

/// Bounded metadata-only syntax, recognized without accepting arbitrary commands.
pub(super) fn is_count_prompt(prompt: &str) -> bool {
    count_total(prompt).is_some()
}

pub(super) fn count_total(prompt: &str) -> Option<usize> {
    let (count, action) = prompt.strip_prefix("Mailbox: ")?.split_once(" unread; ")?;
    let value = count.parse::<u16>().ok()?;
    (super::is_inbox_action(action)
        && (1..=MAX_TOTAL as u16).contains(&value)
        && value.to_string() == count)
        .then_some(usize::from(value))
}

/// The former letter-alias reminder (`Mailbox: b=1 h=2`), read and never
/// produced. Provider history is external and immutable, so reminders already in
/// a transcript must still be recognized, and a receipt stored before the
/// change must be rewritten. Returns the total of its counts.
pub(super) fn legacy_total(prompt: &str) -> Option<usize> {
    let body = prompt.strip_prefix("Mailbox: ")?;
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0usize;
    let valid = prompt.len() <= 51
        && !body.is_empty()
        && body.split(' ').all(|part| {
            let Some((key, count)) = part.split_once('=') else {
                return false;
            };
            let Some(value) = count.parse::<u16>().ok().filter(|value| {
                *value > 0 && usize::from(*value) <= MAX_TOTAL && value.to_string() == count
            }) else {
                return false;
            };
            total += usize::from(value);
            matches!(key, "p" | "h" | "r" | "b" | "d" | "u") && seen.insert(key)
        });
    valid.then_some(total)
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
/// One total in a readable line, for the typed composer reminder and the plain
/// fallbacks, which must stay on one observable row of a 51-column pane.
pub(super) fn count_prompt<'a>(messages: impl Iterator<Item = &'a StoredMessage>) -> String {
    unread_prompt(messages.count())
}

/// The counts spelled out by category, for notice delivery. A notice envelope is
/// multiline and is not held to the one-row width bound of the plain reminder.
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
