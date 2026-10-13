use super::*;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct StreamActivityDiagnosticView {
    pub(crate) reason: String,
    #[serde(flatten)]
    pub(crate) quiet: Option<codex_quiet::QuietMetadata>,
}

pub(super) fn project(diagnostic: &ActivityDiagnosticView) -> Option<StreamActivityDiagnosticView> {
    if !matches!(
        diagnostic.reason.as_str(),
        "completion_evidence_pending"
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
    })
}
