//! Current provider idleness can admit input without inventing a turn outcome.
use super::*;
use crate::activity::{
    ActivityDocument, Confidence, SourceKind, TurnEventKind, controller_state_for_record,
};

/// Ephemeral proof for one bounded Admission, never a persisted turn outcome.
#[derive(Clone, Debug)]
pub(crate) struct StoppedIdle {
    runtime_id: String,
    generation: u64,
    revision: u64,
    turn_id: Option<String>,
    stopped_at: String,
    observed_at: Timestamp,
}

pub(crate) fn stopped_claude_idle(
    context: &CliContext,
    record: &SessionRecord,
    admitted: Option<&StoppedIdle>,
    at: Timestamp,
) -> Option<StoppedIdle> {
    let runtime = record.runtime.as_ref()?;
    let state = controller_state_for_record(context, record)?;
    let dir = session_dir(context, &record.id);
    let document = super::super::read_document(&dir.join(super::super::ACTIVITY_FILE)).ok()?;
    if !super::super::activity_matches_runtime(&document, record) || !exact_stop(&state, &document)
    {
        return None;
    }
    let stopped_at = document.last_provider_event_at.as_deref()?;
    // The shared observation reader owns the Claude-only, runtime/version and
    // post-Stop sample fences.
    let observed_at = completion_observed_at(context, record, state.revision, stopped_at)?;
    let proof = StoppedIdle {
        runtime_id: runtime.launch_id.clone(),
        generation: runtime.generation,
        revision: state.revision,
        turn_id: state.current_turn.as_ref()?.provider_turn_id.clone(),
        stopped_at: stopped_at.to_owned(),
        observed_at: observed_at.parse().ok()?,
    };
    // Age alone cannot revoke this attempt after claim. Re-read the exact Stop
    // and current structured idle every time: newer work, busy/attention or a
    // changed binding still revokes. Never accept a backwards/future sample.
    let bound = admitted.is_some_and(|old| {
        proof.runtime_id == old.runtime_id
            && proof.generation == old.generation
            && proof.revision == old.revision
            && proof.turn_id == old.turn_id
            && proof.stopped_at == old.stopped_at
            && proof.observed_at >= old.observed_at
    });
    (proof.observed_at <= at
        && if admitted.is_some() {
            bound
        } else {
            fresh(&observed_at, at)
        })
    .then_some(proof)
}

fn exact_stop(state: &TurnState, document: &ActivityDocument) -> bool {
    let Some(turn) = state.current_turn.as_ref() else {
        return false;
    };
    state.phase == TurnPhase::Working
        && turn.attention.is_none()
        && state.source.kind == SourceKind::ProviderHook
        && state.source.confidence == Confidence::Observed
        && state
            .diagnostic
            .as_ref()
            .is_some_and(|d| d.reason == "completion_evidence_pending")
        && document.state.revision == state.revision
        && document.last_provider_event_kind == Some(TurnEventKind::StopObserved)
        && document.last_provider_event_provider.as_deref() == Some("claude")
        && document.last_provider_event_turn_id == turn.provider_turn_id
        && document.last_event_at == document.last_provider_event_at
        && document.pending_attention.is_empty()
        && document.overflow_attention.is_none()
}

fn fresh(observed_at: &str, at: Timestamp) -> bool {
    let Ok(observed) = observed_at.parse::<Timestamp>() else {
        return false;
    };
    // Claude Code samples every fifteen seconds; admission permits two samples.
    observed <= at
        && at.as_nanosecond() - observed.as_nanosecond()
            < i128::from(SAMPLE_INTERVAL_SECONDS * 2) * 1_000_000_000
}

#[cfg(test)]
mod tests;
