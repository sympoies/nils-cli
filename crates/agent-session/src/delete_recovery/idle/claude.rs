use super::{Confidence, SourceKind, TurnPhase, active, unknown};
use crate::activity::{bound_claude_state, pending, shadow::claude_agents};
use crate::{CliContext, CliError, SessionRecord};

pub(super) fn require_waiting(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
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

pub(super) fn require_native_idle(record: &SessionRecord) -> Result<(), CliError> {
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
