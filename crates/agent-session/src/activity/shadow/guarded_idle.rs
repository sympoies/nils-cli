//! Current provider idleness can admit input without inventing a turn outcome.
use super::*;
use crate::activity::{Confidence, SourceKind, TurnEventKind, controller_state_for_record};

pub(crate) fn stopped_claude_idle(context: &CliContext, record: &SessionRecord) -> bool {
    if record.agent != "claude" {
        return false;
    }
    let Some(state) = controller_state_for_record(context, record) else {
        return false;
    };
    let Some(turn) = state.current_turn.as_ref() else {
        return false;
    };
    if state.phase != TurnPhase::Working
        || turn.attention.is_some()
        || state.source.kind != SourceKind::ProviderHook
        || state.source.confidence != Confidence::Observed
        || !state
            .diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.reason == "completion_evidence_pending")
    {
        return false;
    }
    let dir = session_dir(context, &record.id);
    let Ok(document) = super::super::read_document(&dir.join(super::super::ACTIVITY_FILE)) else {
        return false;
    };
    // A Stop from another turn, or any intervening accepted event, cannot
    // authorize this turn even if a provider happens to report idle.
    if document.state.revision != state.revision
        || document.last_provider_event_kind != Some(TurnEventKind::StopObserved)
        || document.last_provider_event_provider.as_deref() != Some("claude")
        || document.last_provider_event_turn_id != turn.provider_turn_id
        || document.last_event_at != document.last_provider_event_at
        || !document.pending_attention.is_empty()
        || document.overflow_attention.is_some()
    {
        return false;
    }
    let Some(stopped_at) = document.last_provider_event_at.as_deref() else {
        return false;
    };
    let Some(observed_at) = completion_observed_at(context, record, state.revision, stopped_at)
        .and_then(|at| at.parse::<Timestamp>().ok())
    else {
        return false;
    };
    let at = Timestamp::now();
    // Claude Code's structured observer is sampled every fifteen seconds.
    // Unlike confirmed completion, this admission fact expires after two samples.
    observed_at <= at
        && at.as_nanosecond() - observed_at.as_nanosecond()
            < i128::from(SAMPLE_INTERVAL_SECONDS * 2) * 1_000_000_000
}
