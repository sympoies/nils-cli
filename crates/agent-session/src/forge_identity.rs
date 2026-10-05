//! Immutable initiator/role context, authenticated at the session broker boundary.
use crate::{CliContext, CliError, SessionRecord};
use nils_common::forge_identity::{
    self as identity,
    session::{BINDING_SCHEMA, CONTEXT_KEY, LaunchContext, SessionBinding, SessionReference},
};
use serde_json::{Value, json};

pub(crate) fn error(error: identity::Error) -> CliError {
    CliError::data(error.code, "forge launch binding refused", None)
}
pub(crate) fn context_of(record: &SessionRecord) -> Result<Option<LaunchContext>, CliError> {
    let Some(value) = record
        .lineage
        .as_ref()
        .and_then(|lineage| lineage.extra.get(CONTEXT_KEY))
    else {
        return Ok(None);
    };
    let context: LaunchContext = serde_json::from_value(value.clone())
        .map_err(|_| error(identity::Error::new("identity_session_binding_invalid")))?;
    context.validate().map_err(error)?;
    if context.role != record.role {
        return Err(error(identity::Error::new(
            "identity_session_binding_invalid",
        )));
    }
    Ok(Some(context))
}
pub(crate) fn authenticate_parent(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    let token = crate::coordination::capability_token_from_file(None)?;
    let (authenticated, incarnation) =
        crate::coordination::authenticate_token_observational(context, &record.id, &token)?;
    crate::ensure_same_session_identity(&authenticated, record)?;
    if crate::non_empty_env("AGENT_SESSION_RUNTIME_ID").as_deref() != Some(&incarnation) {
        return Err(error(identity::Error::new(
            "identity_session_binding_mismatch",
        )));
    }
    Ok(())
}
fn reference(reference: &crate::lineage::SessionRef) -> SessionReference {
    SessionReference {
        machine: reference.machine.clone(),
        session_id: reference.session_id.clone(),
        session_created_at: reference.session_created_at.clone(),
        session_incarnation: reference.session_incarnation.clone(),
    }
}
pub(crate) fn projection(
    context: &CliContext,
    args: &crate::cli::BrokerStatusArgs,
) -> Result<Value, CliError> {
    let token = crate::coordination::capability_token_from_file(args.capability_file.as_deref())?;
    let (record, incarnation) =
        crate::coordination::authenticate_token_observational(context, &args.session, &token)?;
    // A copied capability cannot silently retarget the managed caller.
    if let Some(session) = crate::non_empty_env("AGENT_SESSION_ID")
        && (session != record.id
            || crate::non_empty_env("AGENT_SESSION_RUNTIME_ID").as_deref() != Some(&incarnation))
    {
        return Err(error(identity::Error::new(
            "identity_session_binding_mismatch",
        )));
    }
    let launch = context_of(&record)?
        .ok_or_else(|| error(identity::Error::new("identity_session_binding_missing")))?;
    let lineage = record
        .lineage
        .as_ref()
        .ok_or_else(|| error(identity::Error::new("identity_session_binding_missing")))?;
    let binding = SessionBinding {
        schema_version: BINDING_SCHEMA.into(),
        session_id: record.id.clone(),
        session_incarnation: incarnation,
        session_created_at: record.created_at.clone(),
        root: reference(&lineage.root),
        parent: lineage.parent.as_ref().map(reference),
        initiator: launch.initiator,
        role: launch.role,
    };
    binding.validate().map_err(error)?;
    Ok(json!(binding))
}
