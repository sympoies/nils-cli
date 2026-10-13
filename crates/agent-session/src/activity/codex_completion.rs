use super::*;

/// Recover a missing start binding only from the latest accepted progress in
/// this runtime/conversation. A bare completion or Stop proves no new turn.
pub(super) fn restore_progress_turn(
    path: &Path,
    document: &mut ActivityDocument,
    event: &TurnEvent,
) {
    let Some(current) = document.state.current_turn.as_ref() else {
        return;
    };
    let Some(turn_id) = event.provider_turn_id.as_ref() else {
        return;
    };
    if event.provider != "codex"
        || event.kind != TurnEventKind::TurnCompleted
        || event.confidence != Confidence::Authoritative
        || event.source_kind != SourceKind::ProviderHook
        || event.runtime_id != document.runtime_id
        || event.provider_session_id.is_none()
        || event.provider_session_id != document.provider_session_id
        || current.provider_turn_id.as_ref() == Some(turn_id)
        || current.attention.is_some()
        || !document.pending_attention.is_empty()
        || document.overflow_attention.is_some()
        || document.state.phase != TurnPhase::Working
        || document.state.source.kind != SourceKind::ProviderHook
        || document.state.source.provider.as_deref() != Some("codex")
        || document
            .state
            .last_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_ref())
            == Some(turn_id)
        || !matches!(
            document.last_provider_event_kind,
            Some(TurnEventKind::Progress | TurnEventKind::StopObserved)
        )
        || document.last_provider_event_provider.as_deref() != Some("codex")
        || document.last_provider_event_turn_id.as_ref() != Some(turn_id)
    {
        return;
    }
    let Some(progress_at) = current.last_progress_at.as_deref() else {
        return;
    };
    let Ok(started) = current.started_at.parse::<jiff::Timestamp>() else {
        return;
    };
    let Ok(progress) = progress_at.parse::<jiff::Timestamp>() else {
        return;
    };
    if progress < started {
        return;
    }
    let Some(entries) = completion_receipts::retained_entries(path) else {
        return;
    };
    let matching = |entry: &&JournalEntry| {
        entry.applied != Some(false)
            && entry.event.runtime_id == event.runtime_id
            && entry.event.provider == event.provider
            && entry.event.source_kind == SourceKind::ProviderHook
            && entry.event.provider_session_id == event.provider_session_id
            && entry.event.provider_turn_id == event.provider_turn_id
    };
    if entries.iter().filter(matching).any(|entry| {
        (entry.event.event_id != event.event_id
            && matches!(
                entry.event.kind,
                TurnEventKind::TurnCompleted | TurnEventKind::TurnFailed
            ))
            || (entry.event.kind == TurnEventKind::TurnStarted
                && entry
                    .received_at
                    .parse::<jiff::Timestamp>()
                    .is_ok_and(|at| at < started))
    }) {
        return;
    }
    let Some(first_progress) = entries
        .iter()
        .filter(matching)
        .filter(|entry| entry.event.kind == TurnEventKind::Progress)
        .filter_map(|entry| entry.received_at.parse::<jiff::Timestamp>().ok())
        .filter(|at| *at >= started && *at <= progress)
        .min()
    else {
        return;
    };
    if !entries.iter().filter(matching).any(|entry| {
        entry.event.kind == TurnEventKind::Progress && entry.received_at == progress_at
    }) {
        return;
    }
    let progress_at = progress_at.to_string();
    open_provider_turn(document, Some(turn_id.clone()), &first_progress.to_string());
    document
        .state
        .current_turn
        .as_mut()
        .unwrap()
        .last_progress_at = Some(progress_at);
}

/// Older producers could journal a completion without applying it. Re-project
/// that retained receipt only while its correlated progress is still latest.
pub(super) fn recover_completed_snapshot(path: &Path, document: &mut ActivityDocument) {
    let Some(current) = document.state.current_turn.as_ref() else {
        return;
    };
    if document.state.phase != TurnPhase::Working
        || current.provider_turn_id == document.last_provider_event_turn_id
    {
        return;
    }
    let Some(progress_at) = current
        .last_progress_at
        .as_deref()
        .and_then(|at| at.parse::<jiff::Timestamp>().ok())
    else {
        return;
    };
    let Some(entries) = completion_receipts::retained_entries(path) else {
        return;
    };
    let Some(terminal) = entries.iter().rev().find(|entry| {
        entry.event.kind == TurnEventKind::TurnCompleted
            && entry.event.provider == "codex"
            && entry.event.confidence == Confidence::Authoritative
            && entry.event.source_kind == SourceKind::ProviderHook
            && entry.event.runtime_id == document.runtime_id
            && entry.event.provider_session_id == document.provider_session_id
            && entry.event.provider_turn_id == document.last_provider_event_turn_id
            && entry
                .received_at
                .parse::<jiff::Timestamp>()
                .is_ok_and(|at| at >= progress_at)
    }) else {
        return;
    };
    let Ok(completed_at) = terminal.received_at.parse::<jiff::Timestamp>() else {
        return;
    };
    if entries.iter().any(|entry| {
        entry.applied != Some(false)
            && entry
                .received_at
                .parse::<jiff::Timestamp>()
                .is_ok_and(|at| at > completed_at)
            && !(entry.event.kind == TurnEventKind::StopObserved
                && entry.event.runtime_id == terminal.event.runtime_id
                && entry.event.provider == terminal.event.provider
                && entry.event.source_kind == SourceKind::ProviderHook
                && entry.event.provider_session_id == terminal.event.provider_session_id
                && entry.event.provider_turn_id == terminal.event.provider_turn_id)
    }) {
        return;
    }
    restore_progress_turn(path, document, &terminal.event);
    if document
        .state
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_ref())
        == terminal.event.provider_turn_id.as_ref()
        && reduce(document, &terminal.event, &terminal.received_at)
    {
        document.state.semantic_event = Some(SemanticEventView {
            kind: "turn_completed".into(),
            observed_at: terminal.received_at.clone(),
            extra: Map::new(),
        });
        document.state.diagnostic = None;
        document.state.shadow_observation = None;
    }
}
