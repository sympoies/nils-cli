//! A replacement without startup hook evidence can use native mailbox admission.
use super::Admission;
use crate::activity::{self, TurnPhase, TurnState};
use crate::{CliContext, CliError, SessionRecord};

fn replacement_startup(context: &CliContext, record: &SessionRecord) -> Option<TurnState> {
    if record.mode != "interactive"
        || !record
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.kind == "tmux" && runtime.generation > 1)
    {
        return None;
    }
    activity::bound_claude_state(context, record).filter(|state| {
        state.current_turn.is_none() && state.semantic_event.is_none() && state.diagnostic.is_none()
    })
}

pub(crate) fn missing_startup_evidence(context: &CliContext, record: &SessionRecord) -> bool {
    replacement_startup(context, record).is_some_and(|state| state.phase == TurnPhase::Starting)
}

impl Admission<'_> {
    pub(super) fn check_claude_mailbox_idle(&self, stage: &str) -> Result<(), CliError> {
        let missing = || {
            self.refusal(
                "send-completion-evidence-missing",
                "the replacement's startup phase or native idle could not be verified",
                "idle",
                stage,
            )
        };
        if !replacement_startup(self.context, self.record)
            .is_some_and(|state| matches!(state.phase, TurnPhase::Starting | TurnPhase::Waiting))
            || activity::pending::unsettled(self.context, self.record) != Some(false)
            || activity::pending::open_claude_tools(self.context, self.record) != Some(false)
        {
            return Err(missing());
        }
        // Admission brackets the composer snapshot with this shared native
        // probe and repeats both checks before releasing the mailbox input.
        crate::delete_recovery::require_claude_native_idle(self.record).map_err(|error| {
            if error.code() == "session-state-mismatch" {
                self.refusal(
                    "send-session-not-idle",
                    "the native provider is busy or waiting for input",
                    "idle",
                    stage,
                )
            } else {
                missing()
            }
        })
    }
}
