//! Startup readiness shared by provider hooks and observed Codex idle probes.
use super::*;

/// Only an unchanged startup can settle; no turn completion is invented and
/// the preceding runtime's turn outcome remains intact. Native probes supply the
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
    // A fresh, exact-runtime Claude readiness observation supersedes only the
    // carried predecessor's activity quarantine. An auth failure on this
    // runtime already left Starting and cannot enter this transition. Retain
    // the original incident/journal; readiness never claims provider health.
    if record.agent == "claude"
        && let Some(last) = document.state.last_turn.as_mut()
        && last.provider_failure_kind() == Some("authentication")
    {
        last.extra.remove("provider_failure_kind");
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

/// Age makes missing provider evidence explicit; it never authorizes input or
/// changes durable history. A later primary idle observation can still settle.
pub(super) fn project_unverified(record: &SessionRecord, state: &mut TurnState, at: &str) {
    if record.agent != "codex"
        || record
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.kind != "codex_app_server")
        || state.phase != TurnPhase::Starting
        || state.current_turn.is_some()
        || state.diagnostic.is_some()
    {
        return;
    }
    let (Ok(since), Ok(at)) = (
        state.phase_changed_at.parse::<jiff::Timestamp>(),
        at.parse::<jiff::Timestamp>(),
    ) else {
        return;
    };
    let Ok(deadline) = since.checked_add(jiff::SignedDuration::from_secs(30)) else {
        return;
    };
    if at < deadline {
        return;
    }
    state.phase = TurnPhase::Unknown;
    state.source.confidence = Confidence::Inferred;
    state.diagnostic = Some(ActivityDiagnosticView {
        reason: "codex_startup_idle_unverified".into(),
        extra: Map::new(),
    });
}
