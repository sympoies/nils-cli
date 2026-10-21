//! Ordinary runtime-token authentication, separate from scoped worker admission.
use super::{delegation, incarnation, lock_registry, unauthorized, verify_capability_token};
use crate::{CliContext, CliError, SessionRecord, load_session_record};

pub(crate) fn authenticate_token(
    context: &CliContext,
    session_id: &str,
    token: &str,
) -> Result<(SessionRecord, String), CliError> {
    if delegation::is_delegation(token) {
        return delegation::authenticate_token(
            context,
            Some(session_id),
            token,
            delegation::Use::CapabilityOnly,
            true,
        );
    }
    if token.len() < 32 || token.len() > 256 || !token.is_ascii() {
        return Err(unauthorized());
    }
    let record = load_session_record(context, session_id).map_err(|_| unauthorized())?;
    let incarnation = incarnation(&record)?;
    let locked = lock_registry(context)?;
    verify_capability_token(context, &locked.registry, &record, &incarnation, token)?;
    Ok((record, incarnation))
}
