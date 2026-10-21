use super::*;

/// The activity clock. Tests freeze it to cross the quiet window without sleeping.
pub(super) fn now() -> String {
    #[cfg(test)]
    if let Some(frozen) = super::tests::test_clock::frozen_now() {
        return frozen.to_string();
    }
    Zoned::now().timestamp().to_string()
}

pub(crate) fn completion_deadline_for_view(
    context: &CliContext,
    record: &SessionRecord,
) -> Option<jiff::Timestamp> {
    let dir = session_dir(context, &record.id);
    if let Some(runtime) = &record.runtime
        && !matches!(
            runtime_unhealthy_marker(&dir, &runtime.launch_id, runtime.generation),
            RuntimeUnhealthyStatus::Absent
        )
    {
        return None;
    }
    let document = read_document(&dir.join(ACTIVITY_FILE)).ok()?;
    if !activity_matches_runtime(&document, record) || !replay_matches_document(&dir, &document) {
        return None;
    }
    claude_completion_deadline(&document)
}

fn claude_completion_deadline(document: &ActivityDocument) -> Option<jiff::Timestamp> {
    document
        .pending_completion
        .as_ref()?
        .received_at
        .parse::<jiff::Timestamp>()
        .ok()?
        .checked_add(jiff::SignedDuration::from_secs(1))
        .ok()
}

/// Completion is a deterministic projection of the latest qualifying Stop.
/// Reads never write activity; the next accepted event materializes the same
/// settled state before reduction. Any event inside the quiet window cancels it.
pub(super) fn settle_claude_completion(document: &mut ActivityDocument, at: &str) {
    let Some(pending) = document.pending_completion.as_ref() else {
        return;
    };
    let Some(deadline) = claude_completion_deadline(document) else {
        return;
    };
    if !at
        .parse::<jiff::Timestamp>()
        .is_ok_and(|time| time >= deadline)
    {
        return;
    }
    let mut completion = pending.event.clone();
    completion.kind = TurnEventKind::TurnCompleted;
    completion.completion_candidate = false;
    reduce(document, &completion, &deadline.to_string());
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
    // no background tasks is a candidate, and later events cancel it.
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
