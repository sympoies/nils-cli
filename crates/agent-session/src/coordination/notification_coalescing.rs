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

/// Bounded metadata-only syntax, recognized without accepting arbitrary commands.
pub(super) fn is_count_prompt(prompt: &str) -> bool {
    let Some(body) = prompt.strip_prefix("Mailbox: ") else {
        return false;
    };
    let mut seen = std::collections::BTreeSet::new();
    prompt.len() <= 51
        && !body.is_empty()
        && body.split(' ').all(|part| {
            let Some((key, count)) = part.split_once('=') else {
                return false;
            };
            matches!(key, "p" | "h" | "r" | "b" | "d" | "u")
                && seen.insert(key)
                && count
                    .parse::<u16>()
                    .is_ok_and(|value| value > 0 && value <= 256 && value.to_string() == count)
        })
}

fn category_counts<'a>(
    messages: impl Iterator<Item = &'a StoredMessage>,
) -> BTreeMap<(&'static str, &'static str), usize> {
    let mut counts = BTreeMap::new();
    for item in messages {
        let category = match item.category.unwrap_or_default() {
            MessageCategory::Progress => ("p", "progress"),
            MessageCategory::Handoff => ("h", "handoff"),
            MessageCategory::Report => ("r", "report"),
            MessageCategory::Blocker => ("b", "blocker"),
            MessageCategory::Decision => ("d", "decision"),
            MessageCategory::Uncategorized => ("u", "uncategorized"),
        };
        *counts.entry(category).or_default() += 1;
    }
    counts
}

/// Render only unread metadata selected by the owning mailbox transaction.
///
/// The compact form exists for the typed composer reminder, which must stay on
/// one observable row of a 51-column pane.
pub(super) fn count_prompt<'a>(messages: impl Iterator<Item = &'a StoredMessage>) -> String {
    format!(
        "Mailbox: {}",
        category_counts(messages)
            .iter()
            .map(|((kind, _), count)| format!("{kind}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// The same counts spelled out, for notice delivery, which never types into the
/// composer and so has no width bound.
pub(super) fn explained_prompt<'a>(messages: impl Iterator<Item = &'a StoredMessage>) -> String {
    format!(
        "Mailbox: {}; agent-session message inbox",
        category_counts(messages)
            .iter()
            .map(|((_, name), count)| format!("{count} {name}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
