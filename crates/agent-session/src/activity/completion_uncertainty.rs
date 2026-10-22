use super::*;

/// Presentation only: a missing completion never grants control authority.
pub(super) fn project(state: &mut TurnState, at: &str) {
    if state.phase != TurnPhase::Working
        || !state
            .diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.reason == "completion_evidence_pending")
    {
        return;
    }
    let Some(current) = state
        .current_turn
        .as_ref()
        .filter(|turn| turn.attention.is_none())
    else {
        return;
    };
    let mut since = current
        .last_progress_at
        .as_deref()
        .unwrap_or(&current.started_at);
    let mut signal = if current.last_progress_at.is_some() {
        "current_turn.last_progress_at"
    } else {
        "current_turn.started_at"
    };
    // A Stop after a long tool call starts a fresh bounded observation window.
    if let Some(stop) = state
        .semantic_event
        .as_ref()
        .filter(|event| event.kind == "stop_observed")
        && stop.observed_at.parse::<jiff::Timestamp>().ok() > since.parse::<jiff::Timestamp>().ok()
    {
        since = &stop.observed_at;
        signal = "semantic_event.observed_at";
    }
    let (Ok(since), Ok(at)) = (
        since.parse::<jiff::Timestamp>(),
        at.parse::<jiff::Timestamp>(),
    ) else {
        return;
    };
    let Ok(deadline) = since.checked_add(jiff::SignedDuration::from_secs(300)) else {
        return;
    };
    if at < deadline {
        return;
    }
    let diagnostic = state.diagnostic.as_mut().expect("pending diagnostic");
    diagnostic.reason = "completion_evidence_missing".into();
    diagnostic.extra.extend([
        ("quiet_threshold_seconds".into(), json!(300)),
        ("quiet_since".into(), json!(since.to_string())),
        ("quiet_signal".into(), json!(signal)),
    ]);
    state.phase = TurnPhase::Unknown;
    state.phase_changed_at = deadline.to_string();
    state.source.confidence = Confidence::Inferred;
}
