//! Daemon-authored notices use existing mailbox persistence and receipt deduplication.
use super::{Address, Config, Envelope, Origin, Retained, digest_bytes, now_epoch};
use crate::{CliContext, CliError};

fn notice_id(item: &Retained) -> String {
    let digest = digest_bytes(format!("remote-delivery-notice:{}", item.message_id).as_bytes());
    uuid::Uuid::parse_str(&digest[..32])
        .expect("hexadecimal digest")
        .to_string()
}

fn envelope(config: &Config, item: &Retained, sender: &Address) -> Envelope<Origin> {
    let reason = item
        .reason
        .as_deref()
        .and_then(super::relay_reason_code)
        .unwrap_or("remote-delivery-failed");
    let recovery = if item.state == "delivery-unknown" {
        "Delivery is unconfirmed. Inspect the recipient before retrying; a new send may duplicate delivery."
    } else {
        "Review the reason before retrying; use a new idempotency key for a new send."
    };
    let body = format!(
        "Remote message {} ended in {} ({reason}). Inspect it with \
agent-session message delivery --session <sender-session> --message {} --format json. \
{recovery}",
        item.message_id, item.state, item.message_id
    );
    let created = item
        .state_changed_at_epoch
        .expect("selected terminal timestamp");
    Envelope {
        schema_version: super::SERVICE_ENVELOPE_V2.into(),
        message_id: notice_id(item),
        from: Origin::Service(super::super::service::Origin {
            machine: config.machine.clone(),
            service_id: "agent-session-delivery".into(),
            service_generation: "v1".into(),
        }),
        to: sender.clone(),
        body_sha256: digest_bytes(body.as_bytes()),
        body,
        created_at_epoch: created,
        expires_at_epoch: item
            .expires_at_epoch
            .saturating_add(86400)
            .min(created.saturating_add(super::super::mailbox::MAX_EXPIRY_SECS)),
        reply_to: None,
        reply_depth: 0,
        category: Some(crate::cli::MessageCategory::Report),
        forwarding: None,
    }
}

/// Terminal source identities already survive restart without bodies. A fixed
/// notice ID and the existing ingress receipt preserve deduplication through
/// that identity's retention, including after inbox acknowledgement.
pub(super) fn reconcile(context: &CliContext, config: &Config) -> Result<Option<i64>, CliError> {
    let now = now_epoch();
    let candidates = {
        let journal = super::lock_journal(context)?;
        journal.registry.retained.iter()
            .filter(|item| matches!(item.state.as_str(), "rejected" | "delivery-unknown")
                && item.expires_at_epoch.saturating_add(86400) > now
                && item.state_changed_at_epoch.is_some()
                && matches!(&item.sender, Origin::Session(sender) if sender.machine == config.machine))
            .cloned().collect::<Vec<_>>()
    };
    if candidates.is_empty() {
        return Ok(None);
    }
    let pending = {
        let registry = super::super::lock_registry_observational(context)?;
        candidates
            .into_iter()
            .filter_map(|item| {
                let Origin::Session(sender) = &item.sender else {
                    return None;
                };
                let key = super::super::receipt_key(
                    super::RECEIVE_PRINCIPAL,
                    super::RECEIVE_INCARNATION,
                    super::RECEIVE_OPERATION,
                    &notice_id(&item),
                );
                if registry
                    .registry
                    .receipts
                    .get(&key)
                    .is_some_and(|receipt| receipt.expires_at_epoch > now)
                {
                    return None;
                }
                let broker = registry.registry.brokers.get(&sender.session_id);
                if broker.is_some_and(|broker| broker.incarnation != sender.session_incarnation) {
                    return None;
                }
                let ready = broker.is_some_and(|broker| broker.state == "ready");
                Some((item, ready))
            })
            .collect::<Vec<_>>()
    };
    let mut retry = None;
    for (item, ready) in pending {
        let Origin::Session(sender) = &item.sender else {
            continue;
        };
        let record = match crate::load_session_record(context, &sender.session_id) {
            Ok(record) => record,
            Err(error) if matches!(error.code(), "session-not-found" | "message-not-found") => {
                continue;
            }
            Err(error) => return Err(error),
        };
        if super::incarnation(&record)? != sender.session_incarnation {
            continue;
        }
        if !ready {
            retry = Some(now.saturating_add(super::RETRY_SECS));
            continue;
        }
        // receive takes the session lock and rechecks incarnation and readiness.
        // No journal/registry guard spans this local mailbox transaction.
        match super::receive(context, &config.machine, envelope(config, &item, sender)) {
            Ok(_) => return Ok(retry.or_else(|| Some(now_epoch()))),
            Err(error)
                if matches!(
                    error.code(),
                    "session-incarnation-conflict" | "session-not-found"
                ) => {}
            Err(_) => retry = Some(now.saturating_add(super::RETRY_SECS)),
        }
    }
    Ok(retry)
}
