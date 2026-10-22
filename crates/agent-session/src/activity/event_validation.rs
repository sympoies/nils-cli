use super::*;
use jiff::Zoned;

pub(super) fn validate_event(event: &TurnEvent, admission: EventAdmission) -> Result<(), CliError> {
    if let Some(reason) = event.completion_candidate_reason
        && (event.provider != "claude"
            || event.kind != TurnEventKind::StopObserved
            || event.source_kind != SourceKind::ProviderHook
            || event.confidence != Confidence::Observed
            || event.completion_candidate
                != (reason == agent_hook::stop_completion::CompletionCandidateReason::Candidate))
    {
        return Err(CliError::data(
            "activity-completion-candidate-invalid",
            "completion reason requires a consistent observed Claude Stop",
            None,
        ));
    }
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
    for (name, value) in [
        ("event_id", Some(event.event_id.as_str())),
        ("runtime_id", Some(event.runtime_id.as_str())),
        ("provider", Some(event.provider.as_str())),
        ("provider_session_id", event.provider_session_id.as_deref()),
        ("provider_turn_id", event.provider_turn_id.as_deref()),
        ("attention_id", event.attention_id.as_deref()),
    ] {
        if let Some(value) = value
            && (value.is_empty()
                || value.chars().count() > MAX_ID_CHARS
                || value.chars().any(char::is_control))
        {
            return Err(CliError::data(
                "activity-event-field-invalid",
                format!("activity event field {name} is empty, too long, or contains controls"),
                Some(json!({ "field": name })),
            ));
        }
    }
    if !matches!(event.provider.as_str(), "codex" | "claude" | "dsh") {
        return Err(CliError::data(
            "activity-provider-unsupported",
            "activity event provider must be codex, claude, or dsh",
            None,
        ));
    }
    match event.kind {
        TurnEventKind::AttentionRequested => {
            if event.attention_id.is_none() || event.attention_kind.is_none() {
                return Err(CliError::data(
                    "activity-attention-invalid",
                    "attention_requested requires attention_id and attention_kind",
                    None,
                ));
            }
        }
        TurnEventKind::AttentionCleared if event.attention_id.is_none() => {
            return Err(CliError::data(
                "activity-attention-invalid",
                "attention_cleared requires a correlated attention_id",
                None,
            ));
        }
        _ => {}
    }
    if let Some(kind) = event.attention_kind.as_deref()
        && !matches!(
            kind,
            "approval" | "clarification" | "authentication" | "other"
        )
    {
        return Err(CliError::data(
            "activity-attention-kind-invalid",
            "attention kind must be approval, clarification, authentication, or other",
            None,
        ));
    }
    if let Some(reason) = event.failure_reason.as_deref() {
        let reason_allowed = matches!(
            reason,
            "usage_exhausted"
                | "authentication"
                | "organization"
                | "billing"
                | "invalid_request"
                | "service"
                | "max_output_tokens"
                | "unknown"
        ) || (reason == "provider_capacity"
            && admission == EventAdmission::CodexProtocol
            && event.schema_version == CODEX_PROTOCOL_TURN_EVENT_VERSION);
        if event.kind != TurnEventKind::TurnFailed
            || event.confidence != Confidence::Authoritative
            || !reason_allowed
        {
            return Err(CliError::data(
                "activity-failure-reason-invalid",
                "failure_reason requires an authoritative turn_failed event and an allowlisted value",
                None,
            ));
        }
    }
    if let Some(provider_time) = event.provider_time.as_deref() {
        let parsed = provider_time.parse::<jiff::Timestamp>().map_err(|_| {
            CliError::data(
                "activity-provider-time-invalid",
                "provider_time must be a valid RFC 3339 timestamp",
                None,
            )
        })?;
        let skew = parsed
            .as_second()
            .saturating_sub(Zoned::now().timestamp().as_second())
            .unsigned_abs();
        if skew > 24 * 60 * 60 {
            return Err(CliError::data(
                "activity-provider-time-skewed",
                "provider_time is outside the accepted 24-hour skew window",
                None,
            ));
        }
    }
    Ok(())
}
