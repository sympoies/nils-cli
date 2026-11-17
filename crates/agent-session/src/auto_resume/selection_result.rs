//! Selection diagnostics share the existing auto-resume attention projection.
use super::*;

/// Refine discovery failures after the ordinary tick owns scheduling.
/// Repeated faults keep one current reason instead of warning on every poll.
pub(crate) fn record(
    context: &CliContext,
    id: &str,
    launch_id: &str,
    binding: &crate::codex_account::BindingSnapshot,
    error: Option<&CliError>,
) -> Result<(), CliError> {
    let observed = load_session_record(context, id)?;
    let _lock = acquire_session_record_lock(context, &observed.id)?;
    let record = load_session_record(context, &observed.id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, Some(launch_id))
        || crate::codex_account::binding_snapshot(&record) != *binding
    {
        return Ok(());
    }
    let mut state = read_state(context, &record.id, &Timestamp::now().to_string())?;
    let Some(previous) = state.failure_reason.as_deref() else {
        return Ok(());
    };
    let selection_reason =
        previous.starts_with("codex-account-") || previous == "invalid-codex-account";
    let discovery_reason = matches!(
        previous,
        "no_account_available" | "usage_unavailable" | "exhausted_reset_unavailable"
    );
    if !selection_reason && (error.is_none() || !discovery_reason) {
        return Ok(());
    }
    let reason = error.map_or("no_account_available", CliError::code);
    if previous != reason {
        state.failure_reason = Some(reason.to_string());
        write_state(context, &record.id, &state)?;
    }
    Ok(())
}
