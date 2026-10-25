//! Read-only, content-free receipt diagnostics from one locked observation.
use super::*;
use crate::coordination::{
    Registry, capability_token_from_file, lock_registry_observational, notification,
    verify_capability_token,
};

#[derive(Clone, Debug, Serialize)]
pub(super) struct NotificationInspection {
    state: String,
    delivery_state: &'static str,
    generation: u64,
    attempted_generation: u64,
    notified_generation: u64,
    queued_at_epoch: Option<i64>,
    attempted_queued_at_epoch: Option<i64>,
    attempted_at_epoch: Option<i64>,
    updated_at_epoch: Option<i64>,
    next_attempt_at_epoch: Option<i64>,
    last_reason: Option<String>,
    attempt_history: Vec<notification::NotificationAttempt>,
    live_unread_count: usize,
    resolved: bool,
    controller_available: Option<bool>,
    source: ReceiptSource,
    observed_at_epoch: i64,
}

#[derive(Clone, Debug, Serialize)]
struct ReceiptSource {
    registry: &'static str,
    normalization_applied: bool,
    queued_at: &'static str,
}

pub(super) fn status(context: &CliContext, args: BrokerStatusArgs) -> Result<Value, CliError> {
    let record = crate::load_session_record(context, &args.session).map_err(|_| {
        if args.authenticated {
            crate::coordination::unauthorized()
        } else {
            unavailable()
        }
    })?;
    let incarnation = incarnation(&record)?;
    let mut locked = lock_registry_observational(context)?;
    if args.authenticated {
        let token = capability_token_from_file(args.capability_file.as_deref())?;
        if token.len() < 32 || token.len() > 256 || !token.is_ascii() {
            return Err(crate::coordination::unauthorized());
        }
        verify_capability_token(context, &locked.registry, &record, &incarnation, &token)?;
    }
    let broker = locked
        .registry
        .brokers
        .get(&args.session)
        .filter(|broker| broker.incarnation == incarnation)
        .ok_or_else(unavailable)?
        .clone();
    let observed_at_epoch = now_epoch();
    let notification = inspect(
        &mut locked.registry,
        &args.session,
        &incarnation,
        observed_at_epoch,
    );
    let claim = locked
        .registry
        .claims
        .iter()
        .find(|claim| {
            claim.session_id == args.session
                && claim.session_incarnation == incarnation
                && claim.state == "active"
        })
        .map(|claim| ClaimSummary {
            claim_id: claim.claim_id.clone(),
            revision: claim.revision,
            state: claim.state.clone(),
            expires_at: claim.expires_at.clone(),
        });
    let mut operation = OperationSummary::default();
    for lease in &locked.registry.operations {
        if lease.session_id != args.session || lease.session_incarnation != incarnation {
            continue;
        }
        if lease.state == "active" {
            operation.active += 1;
        } else if matches!(lease.state.as_str(), "completing" | "reconcile_pending") {
            operation.uncertain += 1;
        }
    }
    // Normalization affects only this in-memory snapshot. No save, controller
    // probe, receipt claim, or mailbox acknowledgement occurs on this path.
    json_value(BrokerStatus {
        schema_version: BROKER_VERSION.to_string(),
        session_id: args.session.clone(),
        session_incarnation: incarnation.clone(),
        notification,
        state: broker.state,
        generation: broker.generation,
        capability_available: capability_available(
            context,
            &args.session,
            &incarnation,
            &broker.capability_digest,
        ),
        heartbeat_fresh: heartbeat_fresh(
            context,
            &args.session,
            &incarnation,
            broker.heartbeat_epoch,
        ),
        claim,
        operation,
    })
}

fn selected_receipts(registry: &Registry, session: &str, incarnation: &str) -> Value {
    json!(
        registry
            .notifications
            .iter()
            .filter(|(_, receipt)| {
                receipt.target_session_id == session && receipt.target_incarnation == incarnation
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    )
}

fn inspect(
    registry: &mut Registry,
    session: &str,
    incarnation: &str,
    observed_at_epoch: i64,
) -> Option<NotificationInspection> {
    let stored = selected_receipts(registry, session, incarnation);
    // Recover queue provenance from the selected stored receipt metadata alone,
    // using the same migration rules without canonical unread admission.
    let mut stored_registry = Registry {
        notifications: registry
            .notifications
            .iter()
            .filter(|(_, receipt)| {
                receipt.target_session_id == session && receipt.target_incarnation == incarnation
            })
            .map(|(key, receipt)| (key.clone(), receipt.clone()))
            .collect(),
        ..Registry::default()
    };
    notification::normalize_registry(&mut stored_registry, observed_at_epoch);
    let stored_queued_at_epoch = stored_registry
        .notifications
        .values()
        .next()
        .map(|receipt| receipt.queued_at_epoch);
    notification::normalize_registry(registry, observed_at_epoch);
    let receipt = registry.notifications.values().find(|receipt| {
        receipt.target_session_id == session && receipt.target_incarnation == incarnation
    })?;
    let live_unread_count = registry
        .messages
        .iter()
        .filter(|message| {
            message.recipient_session_id == session
                && message.recipient_incarnation == incarnation
                && message.state == "unread"
                && message.expires_at_epoch > observed_at_epoch
        })
        .count();
    let eligible_unread =
        notification::has_live_unread(registry, session, incarnation, observed_at_epoch);
    let queued_at_epoch = timestamp_or_none(receipt.queued_at_epoch);
    Some(NotificationInspection {
        state: receipt.state.clone(),
        delivery_state: if !eligible_unread {
            "resolved"
        } else if receipt.generation <= receipt.notified_generation {
            "submitted"
        } else {
            "queued"
        },
        generation: receipt.generation,
        attempted_generation: receipt.attempted_generation,
        notified_generation: receipt.notified_generation,
        queued_at_epoch,
        attempted_queued_at_epoch: timestamp_or_none(receipt.attempted_queued_at_epoch),
        attempted_at_epoch: timestamp_or_none(receipt.attempted_at_epoch),
        updated_at_epoch: timestamp_or_none(receipt.updated_at_epoch),
        next_attempt_at_epoch: timestamp_or_none(receipt.next_attempt_at_epoch),
        last_reason: receipt
            .last_reason
            .as_deref()
            .map(notification::safe_reason),
        attempt_history: receipt.attempt_history.clone(),
        live_unread_count,
        resolved: !eligible_unread || receipt.generation <= receipt.notified_generation,
        controller_available: None,
        source: ReceiptSource {
            registry: "canonical-local",
            normalization_applied: stored != selected_receipts(registry, session, incarnation),
            queued_at: if queued_at_epoch.is_none() {
                "unavailable"
            } else if stored_queued_at_epoch == Some(receipt.queued_at_epoch) {
                "stored-receipt"
            } else {
                "canonical-unread-admission"
            },
        },
        observed_at_epoch,
    })
}

fn timestamp_or_none(epoch: i64) -> Option<i64> {
    (epoch > 0 && jiff::Timestamp::from_second(epoch).is_ok()).then_some(epoch)
}
