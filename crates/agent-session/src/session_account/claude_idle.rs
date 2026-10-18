//! Shared idle gate and verified restart for queued Claude account intents.

use std::path::Path;

use crate::claude_account::ClaudeAccountView;
use crate::{CliContext, CliError, SessionRecord, load_session_record, write_session_record};

pub(super) fn blocking_reason(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    may_restart: bool,
) -> Option<&'static str> {
    if !may_restart {
        return Some("self_restart_deferred");
    }
    match crate::activity::state_for_view(context, record).map(|state| state.phase) {
        Some(crate::activity::TurnPhase::Waiting) => {}
        Some(crate::activity::TurnPhase::Working) => return Some("turn_busy"),
        Some(crate::activity::TurnPhase::NeedsInput) => return Some("turn_needs_input"),
        Some(crate::activity::TurnPhase::Starting) => return Some("turn_starting"),
        Some(crate::activity::TurnPhase::Unknown) | None => return Some("turn_state_unavailable"),
    }
    match crate::session_status(context, tmux, record).as_str() {
        "running" => None,
        "stopped" => Some("session_stopped"),
        _ => Some("session_status_unknown"),
    }
}

fn store_reason(
    context: &CliContext,
    record: &mut SessionRecord,
    reason: Option<&str>,
) -> Result<(), CliError> {
    if crate::claude_account::set_next_blocking_reason(record, reason)? {
        write_session_record(context, record)?;
    }
    Ok(())
}

/// Re-read under the record lock instead of re-queuing a discovered nickname:
/// cancellation, replacement and a newer account choice must win over a tick.
pub(crate) fn apply_queued(context: &CliContext, id: &str, tmux: &Path) -> Result<(), CliError> {
    let _lock = crate::acquire_session_record_lock(context, id)?;
    let mut record = load_session_record(context, id)?;
    if record.agent != "claude" || !crate::claude_account::has_queued_next(&record) {
        return Ok(());
    }
    if let Some(reason) = blocking_reason(context, &record, tmux, true) {
        return store_reason(context, &mut record, Some(reason));
    }
    if let Err(error) = crate::claude_account::preflight_next(&record) {
        let reason = error
            .details()
            .and_then(|details| details["cause"].as_str())
            .unwrap_or(error.code());
        store_reason(context, &mut record, Some(reason))?;
        return Err(error);
    }
    store_reason(context, &mut record, None)?;
    restart_locked(context, &mut record, tmux).map(|_| ())
}

pub(super) fn restart_locked(
    context: &CliContext,
    record: &mut SessionRecord,
    tmux: &Path,
) -> Result<(Option<String>, Option<ClaudeAccountView>), CliError> {
    let now = jiff::Timestamp::now().to_string();
    let result = crate::validate_resume_metadata(record)
        .map(|_| ())
        .and_then(|()| {
            crate::auto_resume::cancel_for_account_switch_locked(context, &record.id, &now)
        })
        .and_then(|()| crate::stop_session_runtime_locked(context, record, tmux));
    if let Err(error) = result {
        // Stop may have persisted termination evidence. Retain that evidence
        // when adding the reason, rather than writing the pre-stop snapshot.
        let mut current = load_session_record(context, &record.id)?;
        store_reason(context, &mut current, Some(error.code()))?;
        return Err(error);
    }
    super::resume_after_switch_stop(context, &record.id, tmux)
}
