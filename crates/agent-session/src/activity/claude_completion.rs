use super::*;

/// The activity clock; tests freeze it to exercise time-dependent views.
pub(super) fn now() -> String {
    #[cfg(test)]
    if let Some(frozen) = super::tests::test_clock::frozen_now() {
        return frozen.to_string();
    }
    Zoned::now().timestamp().to_string()
}

/// A qualifying Stop needs current structured idle, never elapsed time.
/// Reads do not write activity. The next accepted event materializes the same
/// projection before reduction; intervening accepted work invalidates its revision.
pub(super) fn settle_claude_completion(
    context: &CliContext,
    record: &SessionRecord,
    document: &mut ActivityDocument,
) {
    let Some(pending) = document.pending_completion.as_ref() else {
        return;
    };
    let Some(observed_at) = super::shadow::completion_observed_at(
        context,
        record,
        document.state.revision,
        &pending.received_at,
    ) else {
        return;
    };
    let mut completion = pending.event.clone();
    completion.kind = TurnEventKind::TurnCompleted;
    completion.completion_candidate = false;
    completion.completion_candidate_reason = None;
    reduce(document, &completion, &observed_at);
    document.state.diagnostic = None;
}

fn claude_event_matches_open_turn(document: &ActivityDocument, event: &TurnEvent) -> bool {
    event.provider == "claude"
        && event.source_kind == SourceKind::ProviderHook
        && event.provider_turn_id.as_ref().is_some_and(|turn| {
            document
                .state
                .current_turn
                .as_ref()
                .and_then(|current| current.provider_turn_id.as_ref())
                == Some(turn)
        })
}

pub(super) fn observe_stop(document: &mut ActivityDocument, event: &TurnEvent, at: &str) {
    // Stop ends same-turn permission UI even if another hook continues.
    if claude_event_matches_open_turn(document, event) {
        document
            .pending_attention
            .retain(|pending| pending.kind != "approval");
        refresh_attention(document);
        if document.pending_attention.is_empty() && document.overflow_attention.is_none() {
            document.state.phase = TurnPhase::Working;
        }
    }
    // Matching Stop hooks run concurrently. Only an inactive Stop with
    // no background tasks is a candidate; later accepted events cancel it.
    if event.completion_candidate
        && document.state.current_turn.as_ref().is_some_and(|turn| {
            event.provider_turn_id.is_none()
                || turn.provider_turn_id.is_none()
                || event.provider_turn_id == turn.provider_turn_id
        })
    {
        document.pending_completion = Some(JournalEntry {
            received_at: at.to_string(),
            event: event.clone(),
            applied: Some(true),
        });
    }
}
