//! Scoped credentials are accepted only at explicitly named operation boundaries.
mod cli;
mod transport;
pub(crate) use transport::authenticate_remote_request;
#[cfg(test)]
mod tests;
pub(crate) use cli::{Args, format, name, run};

use super::{Registry, digest_bytes, digest_eq, now_epoch};
use crate::{CliContext, CliError, SessionRecord};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(super) fn registry_version_supported(version: &str) -> bool {
    version.is_empty()
        || matches!(
            version,
            super::REGISTRY_VERSION
                | super::LEGACY_REGISTRY_VERSION
                | super::SUPPRESSION_REGISTRY_VERSION
                | nils_common::coordination_projection::DELEGATION_REGISTRY_VERSION
                | nils_common::coordination_projection::NOTIFICATION_HISTORY_REGISTRY_VERSION
        )
}

pub(super) fn fence_registry(registry: &mut Registry) {
    let delegated = nils_common::coordination_projection::DELEGATION_REGISTRY_VERSION;
    let notification_history =
        nils_common::coordination_projection::NOTIFICATION_HISTORY_REGISTRY_VERSION;
    if registry.schema_version == notification_history
        || registry
            .notifications
            .values()
            .any(|receipt| !receipt.attempt_history.is_empty())
    {
        registry.schema_version = notification_history.into();
    } else if registry.schema_version == delegated
        || !registry.delegations.is_empty()
        || !registry.delegation_audit.is_empty()
    {
        registry.schema_version = delegated.into();
    } else if registry
        .messages
        .iter()
        .any(|message| message.notification_suppressed)
    {
        registry.schema_version = super::SUPPRESSION_REGISTRY_VERSION.into();
    }
}

const PREFIX: &str = "asd1.";
const MAX_AUDIT: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Record {
    id: String,
    session_id: String,
    incarnation: String,
    issuer_machine: String,
    capability_digest: String,
    token_digest: String,
    operations: Vec<cli::Operation>,
    recipients: Vec<String>,
    expires_at_epoch: i64,
    revoked: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Audit {
    delegation_id: String,
    operation: String,
    recipient_digest: Option<String>,
    outcome: String,
    at_epoch: i64,
}

#[derive(Clone, Copy)]
pub(crate) enum Use<'a> {
    StartViaConsole,
    CapabilityOnly,
    LocalMessageSend {
        machine: &'a str,
        recipient: &'a str,
    },
    MessageSend {
        machine: &'a str,
        recipient: &'a str,
    },
}

impl Use<'_> {
    fn operation(self) -> Option<cli::Operation> {
        match self {
            Self::StartViaConsole => Some(cli::Operation::StartViaConsole),
            Self::MessageSend { .. } | Self::LocalMessageSend { .. } => {
                Some(cli::Operation::MessageSend)
            }
            Self::CapabilityOnly => None,
        }
    }
    fn name(self) -> &'static str {
        self.operation()
            .map(cli::Operation::as_str)
            .unwrap_or("capability-only")
    }
    fn recipient(self) -> Option<String> {
        match self {
            Self::StartViaConsole | Self::CapabilityOnly => None,
            Self::MessageSend { machine, recipient }
            | Self::LocalMessageSend { machine, recipient } => {
                Some(format!("{machine}/{recipient}"))
            }
        }
    }
}

fn error(code: &str) -> CliError {
    CliError::data(
        code,
        match code {
            "delegation-expired" => "the worker delegation has expired",
            "delegation-revoked" => "the worker delegation or its caller has been revoked",
            "delegation-incarnation-replaced" => {
                "the delegating session incarnation has been replaced"
            }
            "delegation-scope-denied" => {
                "the worker delegation does not authorize this operation or recipient"
            }
            _ => "the worker delegation is invalid or unavailable",
        },
        None,
    )
}

pub(crate) fn is_delegation(token: &str) -> bool {
    token.starts_with(PREFIX)
}

pub(crate) fn scope_denied() -> CliError {
    error("delegation-scope-denied")
}

fn record<'a>(registry: &'a Registry, token: &str) -> Result<&'a Record, CliError> {
    let id = token
        .strip_prefix(PREFIX)
        .and_then(|tail| tail.split_once('.'))
        .map(|(id, _)| id)
        .ok_or_else(|| error("delegation-unauthorized"))?;
    registry
        .delegations
        .get(id)
        .filter(|record| digest_eq(&record.token_digest, &digest_bytes(token.as_bytes())))
        .ok_or_else(|| error("delegation-unauthorized"))
}

fn verify(
    context: &CliContext,
    registry: &Registry,
    record: &Record,
    session: Option<&str>,
    usage: Use<'_>,
) -> Result<(SessionRecord, String), CliError> {
    if record.revoked {
        return Err(error("delegation-revoked"));
    }
    if record.expires_at_epoch <= now_epoch() {
        return Err(error("delegation-expired"));
    }
    if session.is_some_and(|id| id != record.session_id) {
        return Err(scope_denied());
    }
    let caller = crate::load_session_record(context, &record.session_id)
        .map_err(|_| error("delegation-revoked"))?;
    let incarnation = super::incarnation(&caller).map_err(|_| error("delegation-revoked"))?;
    if incarnation != record.incarnation {
        return Err(error("delegation-incarnation-replaced"));
    }
    let broker = registry
        .brokers
        .get(&record.session_id)
        .ok_or_else(|| error("delegation-revoked"))?;
    if broker.incarnation != incarnation {
        return Err(error("delegation-incarnation-replaced"));
    }
    if broker.state != "ready"
        || !digest_eq(&broker.capability_digest, &record.capability_digest)
        || !super::broker::capability_available(
            context,
            &caller.id,
            &incarnation,
            &record.capability_digest,
        )
    {
        return Err(error("delegation-revoked"));
    }
    if !super::broker::heartbeat_fresh(context, &caller.id, &incarnation, broker.heartbeat_epoch) {
        return Err(error("delegation-revoked"));
    }
    if matches!(usage, Use::LocalMessageSend { machine, .. } if machine != record.issuer_machine) {
        return Err(scope_denied());
    }
    if usage
        .operation()
        .is_none_or(|operation| !record.operations.contains(&operation))
        || usage
            .recipient()
            .is_some_and(|recipient| !record.recipients.contains(&recipient))
    {
        return Err(scope_denied());
    }
    Ok((caller, incarnation))
}

/// Recheck under the operation's commit lock. This never admits a delegation to
/// an ordinary capability consumer and never acquires the registry recursively.
pub(crate) fn revalidate(
    context: &CliContext,
    registry: &Registry,
    session: &str,
    incarnation: &str,
    token: &str,
    usage: Use<'_>,
) -> Result<(), CliError> {
    if !is_delegation(token) {
        let caller =
            crate::load_session_record(context, session).map_err(|_| super::unauthorized())?;
        if super::incarnation(&caller)? != incarnation {
            return Err(super::unauthorized());
        }
        return super::verify_capability_token(context, registry, &caller, incarnation, token);
    }
    let (_, current) = verify(
        context,
        registry,
        record(registry, token)?,
        Some(session),
        usage,
    )?;
    if current != incarnation {
        return Err(error("delegation-incarnation-replaced"));
    }
    Ok(())
}

pub(crate) fn authenticate_file(
    context: &CliContext,
    session: &str,
    path: Option<&Path>,
    usage: Use<'_>,
    audit: bool,
) -> Result<(SessionRecord, String), CliError> {
    let token = super::capability_token_from_file(path)?;
    authenticate_token(context, Some(session), &token, usage, audit)
}

pub(crate) fn authenticate_token(
    context: &CliContext,
    session: Option<&str>,
    token: &str,
    usage: Use<'_>,
    audit: bool,
) -> Result<(SessionRecord, String), CliError> {
    if !is_delegation(token) {
        return match session {
            Some(session) => super::authenticate_token(context, session, token),
            None => super::authenticate_any_token(context, token),
        };
    }
    let mut locked = super::lock_registry(context)?;
    let delegation = record(&locked.registry, token)?;
    let result = verify(context, &locked.registry, delegation, session, usage);
    if audit || result.is_err() {
        let entry = Audit {
            delegation_id: delegation.id.clone(),
            operation: usage.name().into(),
            recipient_digest: usage
                .recipient()
                .map(|recipient| digest_bytes(recipient.as_bytes())),
            outcome: result
                .as_ref()
                .map(|_| "allowed")
                .unwrap_or_else(|error| error.code())
                .into(),
            at_epoch: now_epoch(),
        };
        locked.registry.delegation_audit.push(entry);
        let excess = locked
            .registry
            .delegation_audit
            .len()
            .saturating_sub(MAX_AUDIT);
        locked.registry.delegation_audit.drain(..excess);
        locked.save()?;
    }
    result
}

pub(crate) fn revalidate_file(
    context: &CliContext,
    registry: &Registry,
    session: &str,
    incarnation: &str,
    path: &Path,
    usage: Use<'_>,
) -> Result<(), CliError> {
    revalidate(
        context,
        registry,
        session,
        incarnation,
        &super::capability_token_from_file(Some(path))?,
        usage,
    )
}
