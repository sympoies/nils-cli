//! Claude Code 2.1.317 has no documented interactive interrupt/denial outcome.
//! Structured idle without completion evidence is uncertainty, never a timer
//! inference. Keep this presentation separate from reducer/control state so a
//! later Stop, failure or idle notification can still settle the same turn.

use super::{SessionRecord, ShadowDocument, TurnPhase, TurnState};
use crate::activity::{ActivityDiagnosticView, Confidence, SourceKind};

pub(super) fn project(record: &SessionRecord, cached: &ShadowDocument, state: &mut TurnState) {
    if record.agent != "claude"
        || cached.activity_revision != Some(state.revision)
        || cached.observation.rule_id != "claude_agents_idle"
        || cached.observation.projection != "waiting"
        || !super::is_recent(
            &cached.observation.observed_at,
            super::SAMPLE_INTERVAL_SECONDS * 2,
        )
        || state.current_turn.is_none()
        || !matches!(
            state.phase,
            TurnPhase::Working | TurnPhase::NeedsInput | TurnPhase::Unknown
        )
    {
        return;
    }
    state.phase = TurnPhase::Unknown;
    // Preserve the semantic clock: repeated idle samples are not new turn
    // transitions, and cannot generate duplicate stream wakeups.
    state.source.kind = SourceKind::ConsoleObservation;
    state.source.confidence = Confidence::Observed;
    state.diagnostic = Some(ActivityDiagnosticView {
        reason: "claude_turn_outcome_unknown".into(),
        extra: serde_json::Map::new(),
    });
}
