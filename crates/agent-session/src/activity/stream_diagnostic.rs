use super::*;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct StreamActivityDiagnosticView {
    pub(crate) reason: String,
    #[serde(flatten)]
    pub(crate) quiet: Option<codex_quiet::QuietMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completion_candidate_reason:
        Option<agent_hook::stop_completion::CompletionCandidateReason>,
}

pub(super) fn project(diagnostic: &ActivityDiagnosticView) -> Option<StreamActivityDiagnosticView> {
    if !matches!(
        diagnostic.reason.as_str(),
        "completion_evidence_pending"
            | "completion_evidence_missing"
            | "attention_authority_mismatch"
            | "provider_projection_unavailable"
            | "runtime_activity_unhealthy"
            | "activity_state_unavailable"
            | "interrupted_suspected"
            | "codex_progress_quiet"
    ) {
        return None;
    }
    Some(StreamActivityDiagnosticView {
        reason: diagnostic.reason.clone(),
        quiet: codex_quiet::metadata(diagnostic),
        completion_candidate_reason: super::completion_diagnostic::candidate_reason(
            &diagnostic.extra,
        ),
    })
}
