//! Body-free quota evidence and recipient notification signals, outside message capacity.
use serde::{Deserialize, Serialize};

use super::{Registry, notification, remote};
use crate::cli::MessageCategory;

const MAX_EVENTS: usize = 256;
const MAX_RECIPIENT_EVENTS: usize = 32;
const RETENTION_SECONDS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Rejection {
    sender: remote::Origin,
    recipient: remote::Address,
    category: MessageCategory,
    occurred_at: String,
    occurred_at_epoch: i64,
    reason_code: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(super) struct State {
    above_threshold: bool,
    signal_generation: u64,
    signal_expires_at_epoch: i64,
}

pub(super) fn pending(receipt: &notification::NotificationReceipt, now: i64) -> bool {
    receipt.quota.as_ref().is_some_and(|quota| {
        quota.signal_generation > receipt.notified_generation && quota.signal_expires_at_epoch > now
    })
}

pub(super) fn has_pending(registry: &Registry, session: &str, incarnation: &str, now: i64) -> bool {
    registry.notifications.values().any(|receipt| {
        receipt.target_session_id == session
            && receipt.target_incarnation == incarnation
            && pending(receipt, now)
    })
}

fn signal(registry: &mut Registry, session: &str, incarnation: &str, now: i64) {
    notification::schedule(registry, session, incarnation, now);
    let receipt = registry
        .notifications
        .values_mut()
        .find(|receipt| {
            receipt.target_session_id == session && receipt.target_incarnation == incarnation
        })
        .expect("scheduled recipient");
    let quota = receipt.quota.get_or_insert_with(State::default);
    quota.signal_generation = receipt.generation;
    quota.signal_expires_at_epoch = now.saturating_add(RETENTION_SECONDS);
    receipt.urgent_generation = receipt.generation;
    receipt.prompt = Some(nils_common::coordination_projection::MAILBOX_QUOTA_ALERT_NOTICE.into());
}

pub(super) fn rejected(
    registry: &mut Registry,
    sender: remote::Origin,
    recipient: remote::Address,
    category: Option<MessageCategory>,
    now: i64,
) {
    let session = recipient.session_id.clone();
    let incarnation = recipient.session_incarnation.clone();
    registry.quota_rejections.push(Rejection {
        sender,
        recipient,
        category: category.unwrap_or_default(),
        occurred_at: super::timestamp(now),
        occurred_at_epoch: now,
        reason_code: "quota-exceeded".into(),
    });
    // Keep the newest recipient events without allowing one full mailbox to
    // consume the entire host-wide evidence window.
    let mut retained = 0;
    registry.quota_rejections.reverse();
    registry.quota_rejections.retain(|event| {
        if event.recipient.session_id != session {
            return true;
        }
        retained += 1;
        retained <= MAX_RECIPIENT_EVENTS
    });
    registry.quota_rejections.reverse();
    let excess = registry.quota_rejections.len().saturating_sub(MAX_EVENTS);
    registry.quota_rejections.drain(..excess);
    signal(registry, &session, &incarnation, now);
}

/// Destination ingress has already authenticated and fenced the exact target.
pub(super) fn admit_remote(
    context: &crate::CliContext,
    locked: &mut super::LockedRegistry,
    envelope: &remote::Envelope<remote::Origin>,
    session: &str,
    now: i64,
    now_millis: i64,
) -> Result<u8, crate::CliError> {
    let percent = super::notification_policy::mailbox_alert_percent(context, session)?;
    rearm(&mut locked.registry, session, percent, now);
    if let Err(error) = super::mailbox::admit_message(
        &mut locked.registry,
        &envelope.from.stored_id(),
        session,
        envelope.body.len(),
        envelope.reply_to.as_deref(),
        now,
        now_millis,
    ) {
        if error.code() == "quota-exceeded" {
            let mut recipient = envelope.to.clone();
            recipient.session_id = session.into();
            rejected(
                &mut locked.registry,
                envelope.from.clone(),
                recipient,
                envelope.category,
                now,
            );
            locked.save()?;
        }
        return Err(error);
    }
    Ok(percent)
}

fn above(registry: &Registry, session: &str, percent: u8, now: i64) -> bool {
    let mut count = 0;
    let mut bytes = 0;
    for message in &registry.messages {
        if message.recipient_session_id == session
            && super::mailbox::counts_toward_quota(message, now)
        {
            count += 1;
            bytes += message.body_bytes;
        }
    }
    count >= (super::mailbox::MAX_SESSION_MESSAGES * usize::from(percent)).div_ceil(100)
        || bytes >= (super::mailbox::MAX_SESSION_BYTES * usize::from(percent)).div_ceil(100)
}

/// Observe the pre-admission usage so ack/expiry below the boundary re-arms
/// even when the next accepted message immediately crosses it again.
pub(super) fn rearm(registry: &mut Registry, session: &str, percent: u8, now: i64) {
    if !above(registry, session, percent, now) {
        for receipt in registry
            .notifications
            .values_mut()
            .filter(|r| r.target_session_id == session)
        {
            if let Some(quota) = &mut receipt.quota {
                quota.above_threshold = false;
            }
        }
    }
}

pub(super) fn admitted(
    registry: &mut Registry,
    session: &str,
    incarnation: &str,
    percent: u8,
    now: i64,
    outcome: &mut serde_json::Value,
) {
    if !above(registry, session, percent, now) {
        return;
    }
    let already = registry
        .notifications
        .values()
        .find(|r| r.target_session_id == session && r.target_incarnation == incarnation)
        .and_then(|r| r.quota.as_ref())
        .is_some_and(|q| q.above_threshold);
    if !already {
        signal(registry, session, incarnation, now);
        let quota = registry
            .notifications
            .values_mut()
            .find(|r| r.target_session_id == session && r.target_incarnation == incarnation)
            .and_then(|r| r.quota.as_mut())
            .expect("quota signal");
        quota.above_threshold = true;
        outcome["notification"] = serde_json::to_value(notification::projection_for(
            registry,
            session,
            incarnation,
            false,
        ))
        .expect("notification projection");
    }
}

pub(super) fn prune(registry: &mut Registry, now: i64) -> bool {
    let before = registry.quota_rejections.len();
    registry
        .quota_rejections
        .retain(|event| event.occurred_at_epoch > now.saturating_sub(RETENTION_SECONDS));
    registry.quota_rejections.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn address(id: &str) -> remote::Address {
        remote::Address {
            machine: "fixture".into(),
            session_id: id.into(),
            session_incarnation: "incarnation".into(),
        }
    }

    #[test]
    fn rejection_history_has_recipient_global_and_time_bounds() {
        let mut registry = Registry::default();
        rejected(
            &mut registry,
            address("sender").into(),
            address("recipient"),
            None,
            1,
        );
        let event = registry.quota_rejections[0].clone();
        registry.quota_rejections = vec![event.clone(); MAX_RECIPIENT_EVENTS + 1];
        rejected(
            &mut registry,
            address("sender").into(),
            address("recipient"),
            None,
            2,
        );
        assert_eq!(registry.quota_rejections.len(), MAX_RECIPIENT_EVENTS);
        registry.quota_rejections = (0..MAX_EVENTS)
            .map(|index| {
                let mut event = event.clone();
                event.recipient.session_id = format!("recipient-{index}");
                event
            })
            .collect();
        rejected(
            &mut registry,
            address("sender").into(),
            address("recipient"),
            None,
            3,
        );
        assert_eq!(registry.quota_rejections.len(), MAX_EVENTS);
        assert_eq!(
            registry.quota_rejections[0].recipient.session_id,
            "recipient-1"
        );
        assert!(prune(&mut registry, RETENTION_SECONDS + 2));
        assert_eq!(registry.quota_rejections.len(), 1);
        assert!(prune(&mut registry, RETENTION_SECONDS + 3));
        assert!(registry.quota_rejections.is_empty());
    }

    #[test]
    fn quota_signal_without_unread_mail_is_deliverable_and_fenced_until_expiry() {
        let mut registry = Registry::default();
        rejected(
            &mut registry,
            address("sender").into(),
            address("recipient"),
            None,
            100,
        );
        let candidates = notification::deliverable_candidates(&mut registry, 101);
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].prompt.as_deref(),
            Some(nils_common::coordination_projection::MAILBOX_QUOTA_ALERT_NOTICE)
        );
        super::super::delegation::fence_registry(&mut registry);
        assert_eq!(
            registry.schema_version,
            nils_common::coordination_projection::QUOTA_REGISTRY_VERSION
        );
        assert!(
            notification::deliverable_candidates(&mut registry, 100 + RETENTION_SECONDS).is_empty()
        );
    }
}
