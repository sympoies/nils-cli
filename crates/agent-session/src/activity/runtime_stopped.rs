use super::*;

/// Plain Codex has no structured failure completion. An observed runtime exit
/// ends its working presentation without inventing a provider turn outcome.
/// Apply to control views too: a stopped runtime cannot be an active turn.
pub(super) fn project(context: &CliContext, record: &SessionRecord, state: &mut TurnState) {
    if record.agent != "codex" || !matches!(state.phase, TurnPhase::Working | TurnPhase::Unknown) {
        return;
    }
    // Most live sessions have no exit marker; avoid scanning their lifecycle journal.
    if !context
        .state_dir
        .join("lifecycle")
        .join(&record.id)
        .join("exit-observed.json")
        .is_file()
    {
        return;
    }
    let Some(at) = crate::lifecycle::stopped_observed_at(context, record) else {
        return;
    };
    state.phase = TurnPhase::Unknown;
    state.phase_changed_at = at;
    state.source = runtime_source();
    state.diagnostic = Some(ActivityDiagnosticView {
        reason: "codex_runtime_stopped".into(),
        extra: Map::new(),
    });
    state.shadow_observation = None;
}
