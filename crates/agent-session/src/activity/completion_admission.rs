use super::*;

/// Receipt and application are separate: rejected completions cannot cancel a
/// pending Stop candidate or replace the current turn's accepted presentation.
pub(super) fn applies(document: &ActivityDocument, event: &TurnEvent) -> bool {
    let requires_exact_open_turn = event.provider == AgentKind::Codex.as_str()
        && event.kind == TurnEventKind::TurnCompleted
        && event.confidence == Confidence::Authoritative
        && event.source_kind == SourceKind::ProviderHook
        && event.provider_turn_id.is_some();
    if requires_exact_open_turn {
        return document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_ref())
            == event.provider_turn_id.as_ref();
    }
    if let Some(current) = document.state.current_turn.as_ref() {
        return event.provider_turn_id.is_none()
            || current.provider_turn_id.is_none()
            || event.provider_turn_id == current.provider_turn_id
            || reconciles_claude_idle(document, event, current);
    }
    if let (Some(event_turn_id), Some(last_turn_id)) = (
        event.provider_turn_id.as_ref(),
        document
            .state
            .last_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_ref()),
    ) {
        return event_turn_id == last_turn_id;
    }
    true
}

/// Claude idle_prompt is the observed hook completion. A local /compact can
/// change its prompt ID without announcing a new turn. The exact current Stop
/// plus an idle signal in the bound conversation closes the stopped turn; a
/// known older completion or any later accepted work/attention cannot do so.
pub(super) fn reconciles_claude_idle(
    document: &ActivityDocument,
    event: &TurnEvent,
    current: &CurrentTurn,
) -> bool {
    let (Some(current_id), Some(completion_id), Some(conversation_id)) = (
        current.provider_turn_id.as_ref(),
        event.provider_turn_id.as_ref(),
        document.provider_session_id.as_ref(),
    ) else {
        return false;
    };
    completion_id != current_id
        && event.kind == TurnEventKind::TurnCompleted
        && event.provider == AgentKind::Claude.as_str()
        && event.runtime_id == document.runtime_id
        && event.source_kind == SourceKind::ProviderHook
        && event.confidence == Confidence::Observed
        && event.provider_session_id.as_ref() == Some(conversation_id)
        && document.pending_completion.as_ref().is_some_and(|pending| {
            pending.event.kind == TurnEventKind::StopObserved
                && pending.event.completion_candidate
                && pending.event.runtime_id == document.runtime_id
                && pending.event.provider == AgentKind::Claude.as_str()
                && pending.event.provider_session_id.as_ref() == Some(conversation_id)
                && pending.event.provider_turn_id.as_ref() == Some(current_id)
                && pending.event.source_kind == SourceKind::ProviderHook
                && pending.event.confidence == Confidence::Observed
        })
        && document.state.phase == TurnPhase::Working
        && document.state.source.kind == SourceKind::ProviderHook
        && document.state.source.provider.as_deref() == Some(AgentKind::Claude.as_str())
        && document.state.source.confidence == Confidence::Observed
        && document.last_provider_event_kind == Some(TurnEventKind::StopObserved)
        && document.last_provider_event_provider.as_deref() == Some(AgentKind::Claude.as_str())
        && document.last_provider_event_turn_id.as_ref() == Some(current_id)
        && document.last_event_at == document.last_provider_event_at
        && document.last_semantic_event_at == document.last_provider_event_at
        && document
            .state
            .semantic_event
            .as_ref()
            .is_some_and(|semantic| {
                semantic.kind == "stop_observed"
                    && Some(semantic.observed_at.as_str())
                        == document.last_provider_event_at.as_deref()
            })
        && document
            .state
            .diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.reason == "completion_evidence_pending")
        && document.pending_attention.is_empty()
        && document.overflow_attention.is_none()
        && current.attention.is_none()
        && document
            .state
            .last_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_ref())
            != Some(completion_id)
}

pub(super) fn close_turn(document: &mut ActivityDocument, event: &TurnEvent, at: &str) {
    let current = document.state.current_turn.take();
    let extra = if let Some(current) = current.as_ref() {
        current.extra.clone()
    } else if let (Some(event_turn_id), Some(last_turn)) = (
        event.provider_turn_id.as_ref(),
        document.state.last_turn.as_ref(),
    ) {
        if last_turn.provider_turn_id.as_ref() == Some(event_turn_id) {
            last_turn.extra.clone()
        } else {
            Map::new()
        }
    } else {
        Map::new()
    };
    document.state.last_turn = Some(LastTurn {
        // Reconciliation completes A; the raw idle(B) remains in the journal.
        provider_turn_id: current
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.clone())
            .or_else(|| event.provider_turn_id.clone()),
        started_at: current.map(|turn| turn.started_at),
        completed_at: at.to_string(),
        outcome: if event.kind == TurnEventKind::TurnFailed {
            "failed".to_string()
        } else {
            "completed".to_string()
        },
        extra: if matches!(
            event.failure_reason.as_deref(),
            Some("provider_capacity" | "authentication")
        ) {
            let mut extra = extra;
            extra.insert(
                "provider_failure_kind".to_string(),
                json!(event.failure_reason),
            );
            extra
        } else {
            let mut extra = extra;
            extra.remove("provider_failure_kind");
            extra
        },
    });
    document.pending_attention.clear();
    document.overflow_attention = None;
    document.state.phase = TurnPhase::Waiting;
}
