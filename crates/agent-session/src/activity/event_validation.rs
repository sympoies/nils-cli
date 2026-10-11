use super::*;

pub(super) fn validate_event_prefix(
    event: &TurnEvent,
    admission: EventAdmission,
) -> Result<(), CliError> {
    if event.completion_candidate
        && !(event.provider == "claude"
            && event.kind == TurnEventKind::StopObserved
            && event.source_kind == SourceKind::ProviderHook
            && event.confidence == Confidence::Observed)
    {
        return Err(CliError::data(
            "activity-completion-candidate-invalid",
            "completion candidates require an observed Claude provider Stop",
            None,
        ));
    }
    let schema_supported = event.schema_version == TURN_EVENT_VERSION
        || (admission == EventAdmission::CodexProtocol
            && event.schema_version == CODEX_PROTOCOL_TURN_EVENT_VERSION);
    if !schema_supported {
        return Err(CliError::data(
            "unsupported-turn-event-version",
            format!(
                "unsupported turn event schema {}; expected {TURN_EVENT_VERSION}",
                event.schema_version
            ),
            None,
        ));
    }
    Ok(())
}
