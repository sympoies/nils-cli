use super::{
    ActivityFence, Confidence, SourceKind, TurnPhase, acquire_activity, active, check_composer,
    unknown,
};
use crate::activity::{bound_claude_state, pending, shadow::claude_agents};
use crate::{
    CliContext, CliError, PANE_OBSERVATION_COMMAND_TIMEOUT, SessionRecord, exact_tmux_target,
    run_output_with_timeout,
};
use serde_json::json;
use std::{path::Path, process::Command, time::Instant};

/// Held under the caller's session record lock through verified teardown.
pub(crate) struct Guard {
    _activity: ActivityFence,
}

impl Guard {
    pub(crate) fn check_before_teardown(
        &self,
        tmux: &Path,
        record: &SessionRecord,
    ) -> Result<(), CliError> {
        // Repeat the attached-client check as late as possible. The remaining
        // check-to-teardown interval is the same residual as idle retirement.
        require_detached(tmux, record)
    }
}

/// The caller already established the exact running managed Claude runtime.
/// Neither probing nor refusing this admission changes the pane.
pub(crate) fn acquire(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
) -> Result<Guard, CliError> {
    let activity = acquire_activity(context, record)?;
    require_waiting(context, record)?;
    require_native_idle(record)?;
    let deadline = Instant::now() + crate::send::guarded_hold_budget(record, false);
    check_composer(context, record, tmux, deadline)?;
    // Bracket the final native query with captures to catch a late draft.
    require_native_idle(record)?;
    check_composer(context, record, tmux, deadline)?;
    require_detached(tmux, record)?;
    Ok(Guard {
        _activity: activity,
    })
}

fn require_waiting(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    if pending::open_claude_tools(context, record).ok_or_else(|| unknown(record))? {
        return Err(active(record, "provider-tool-pending"));
    }
    let state = bound_claude_state(context, record).ok_or_else(|| unknown(record))?;
    match state.phase {
        TurnPhase::Waiting
            if state.current_turn.is_none()
                && state.source.kind == SourceKind::ProviderHook
                && state.source.provider.as_deref() == Some("claude")
                && matches!(
                    state.source.confidence,
                    Confidence::Authoritative | Confidence::Observed
                ) =>
        {
            Ok(())
        }
        TurnPhase::Working | TurnPhase::NeedsInput | TurnPhase::Starting => {
            Err(active(record, "working"))
        }
        _ => Err(unknown(record)),
    }
}

fn require_native_idle(record: &SessionRecord) -> Result<(), CliError> {
    match claude_agents::fresh_state_for(record).map_err(|_| unknown(record))? {
        claude_agents::AgentState::Idle => Ok(()),
        claude_agents::AgentState::Busy => Err(active(record, "working")),
        claude_agents::AgentState::Waiting(claude_agents::WaitingFor::PermissionPrompt) => {
            Err(active(record, "permission-prompt"))
        }
        claude_agents::AgentState::Waiting(claude_agents::WaitingFor::Other) => {
            Err(active(record, "needs-input"))
        }
    }
}

/// A human can only type into the pane through an attached tmux client.
fn require_detached(tmux: &Path, record: &SessionRecord) -> Result<(), CliError> {
    let mut command = Command::new(tmux);
    command
        .args(["list-clients", "-F", "#{client_name}", "-t"])
        .arg(exact_tmux_target(&record.tmux_session));
    let output = run_output_with_timeout(command, PANE_OBSERVATION_COMMAND_TIMEOUT)
        .ok()
        .filter(|output| output.status.success())
        .ok_or_else(|| unknown(record))?;
    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    Err(CliError::data(
        "session-client-attached",
        "a tmux client is attached to the session",
        Some(json!({"id": record.id})),
    ))
}
