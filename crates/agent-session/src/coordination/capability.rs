//! Ordinary runtime-token authentication, separate from scoped worker admission.
use super::{delegation, incarnation, lock_registry, unauthorized, verify_capability_token};
use crate::{CliContext, CliError, SessionRecord, load_session_record};

/// Local refusal diagnostics contain fixed codes only, never authority selectors.
/// Keep the existing outer code so callers continue to fail closed.
pub(super) fn refusal(reason_code: &'static str) -> CliError {
    let next_action = match reason_code {
        "capability-file-missing" | "capability-file-invalid" | "capability-mismatch" => {
            "verify-session-capability-file"
        }
        _ => "preserve-session-and-request-replacement",
    };
    let mut error = unauthorized().with_hint(format!(
        "{reason_code}: verify the runtime-supplied capability file with `agent-session broker identity`; if authority no longer matches, preserve the session, signed work and unread mailbox and ask the coordinator for a replacement session and sender resend; do not edit authority files or restart a live pane to repair this refusal",
    ));
    *error.details_mut() = Some(serde_json::json!({
        "reason_code": reason_code,
        "retryable": false,
        "next_action": next_action,
        "recovery": {
            "action": next_action,
            "preserve_session": true,
            "preserve_mailbox": true
        }
    }));
    error
}

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
        return Err(refusal("capability-file-invalid"));
    }
    let record = load_session_record(context, session_id).map_err(|_| unauthorized())?;
    let incarnation = incarnation(&record)?;
    let locked = lock_registry(context)?;
    verify_capability_token(context, &locked.registry, &record, &incarnation, token)?;
    Ok((record, incarnation))
}
