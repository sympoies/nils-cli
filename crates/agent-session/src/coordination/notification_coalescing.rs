//! Destination-owned routine windows; urgency follows authenticated metadata.
use super::{NotificationReceipt, Registry, receipt_key};
use crate::cli::MessageCategory;
use crate::coordination::mailbox::StoredMessage;
use std::collections::BTreeMap;

pub(super) fn due(receipt: &NotificationReceipt, now: i64) -> bool {
    receipt.routine_interval_seconds == 0
        || receipt.urgent_generation > receipt.notified_generation
        || receipt.routine_last_submitted_at_epoch == 0
        || now.saturating_sub(receipt.routine_last_submitted_at_epoch)
            >= receipt.routine_interval_seconds
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
    let mut counts = BTreeMap::<String, usize>::new();
    for item in registry
        .messages
        .iter()
        .filter(|item| {
            item.recipient_session_id == message.recipient_session_id
                && item.recipient_incarnation == message.recipient_incarnation
                && item.state == "unread"
                && !item.notification_suppressed
                && item.expires_at_epoch > now
        })
        .chain(std::iter::once(message))
    {
        let category = match item.category.unwrap_or_default() {
            MessageCategory::Progress => "p",
            MessageCategory::Handoff => "h",
            MessageCategory::Report => "r",
            MessageCategory::Blocker => "b",
            MessageCategory::Decision => "d",
            MessageCategory::Uncategorized => "u",
        };
        *counts.entry(category.to_owned()).or_default() += 1;
    }
    if let Some(receipt) = registry.notifications.get_mut(&receipt_key(
        &message.recipient_session_id,
        &message.recipient_incarnation,
    )) {
        receipt.routine_interval_seconds = interval;
        if urgent {
            receipt.urgent_generation = receipt.generation;
        }
        receipt.prompt = (interval > 0).then(|| {
            format!(
                "Mailbox: {}",
                counts
                    .iter()
                    .map(|(kind, count)| format!("{kind}={count}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        });
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
