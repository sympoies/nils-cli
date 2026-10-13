use super::*;

/// A matching Stop records observation, not new work. Keep the authority of
/// the completed Codex turn so that the idle projection remains wake-eligible.
/// The Stop itself still enters the observed-event journal and replay horizon.
pub(super) fn source_for_event(document: &ActivityDocument, event: &TurnEvent) -> TurnSource {
    let preserves_completion = event.provider == AgentKind::Codex.as_str()
        && event.kind == TurnEventKind::StopObserved
        && event.source_kind == SourceKind::ProviderHook
        && event.confidence == Confidence::Observed
        && document.state.phase == TurnPhase::Waiting
        && document.state.current_turn.is_none()
        && document.state.source.kind == SourceKind::ProviderHook
        && document.state.source.provider.as_deref() == Some(AgentKind::Codex.as_str())
        && document.state.source.confidence == Confidence::Authoritative
        && event.provider_turn_id.as_ref().is_some_and(|id| {
            document.state.last_turn.as_ref().is_some_and(|last| {
                last.outcome == "completed" && last.provider_turn_id.as_ref() == Some(id)
            })
        });
    if preserves_completion {
        return document.state.source.clone();
    }
    let mut source = provider_source(event);
    source.extra = document.state.source.extra.clone();
    source
}
