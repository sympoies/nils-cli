//! Startup readiness shared by provider hooks and observed Codex idle probes.
use super::*;

/// Only an unchanged startup can settle; no turn completion is invented and
/// the preceding runtime's last turn remains intact. Native probes supply the
/// revision they observed before I/O; Claude's SessionStart is itself the event.
pub(crate) fn settle_starting(
    context: &CliContext,
    record: &SessionRecord,
    expected_revision: Option<u64>,
) -> Result<bool, CliError> {
    let Some(runtime) = record.runtime.as_ref() else {
        return Ok(false);
    };
    let dir = session_dir(context, &record.id);
    let _lock = acquire_lock(&dir)?;
    if !matches!(
        runtime_unhealthy_marker(&dir, &runtime.launch_id, runtime.generation),
        RuntimeUnhealthyStatus::Absent
    ) {
        return Ok(false);
    }
    let path = dir.join(ACTIVITY_FILE);
    let Ok(mut document) = read_document(&path) else {
        return Ok(false);
    };
    if document.runtime_id != runtime.launch_id
        || document.runtime_generation != runtime.generation
        || !replay_matches_document(&dir, &document)
        || document.state.phase != TurnPhase::Starting
        || document.state.current_turn.is_some()
        || !document.pending_attention.is_empty()
        || document.overflow_attention.is_some()
        || document.pending_journal.is_some()
        || document.pending_completion.is_some()
        || expected_revision.is_some_and(|revision| document.state.revision != revision)
        || (expected_revision.is_some() && pending::event_pending(context, &record.id)?)
    {
        return Ok(false);
    }
    document.state.phase = TurnPhase::Waiting;
    document.state.phase_changed_at = now();
    document.state.revision = document.state.revision.saturating_add(1);
    document.state.source = TurnSource {
        kind: SourceKind::ProviderHook,
        provider: Some(record.agent.clone()),
        confidence: Confidence::Authoritative,
        extra: Map::new(),
    };
    document.state.diagnostic = None;
    write_document(&path, &mut document)?;
    Ok(true)
}
