use super::*;

pub(super) fn pending(event: &TurnEvent) -> ActivityDiagnosticView {
    let mut extra = Map::new();
    if let Some(reason) = event.completion_candidate_reason {
        extra.insert("completion_candidate_reason".into(), json!(reason));
    }
    ActivityDiagnosticView {
        reason: "completion_evidence_pending".into(),
        extra,
    }
}

pub(super) fn candidate_reason(
    fields: &Map<String, Value>,
) -> Option<agent_hook::stop_completion::CompletionCandidateReason> {
    serde_json::from_value(fields.get("completion_candidate_reason")?.clone()).ok()
}
