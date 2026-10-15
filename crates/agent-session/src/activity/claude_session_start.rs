use super::*;

/// Route a primary Claude session hook to the model observer and, for a
/// SessionStart(startup|resume), to the provider-idle readiness projection.
/// Returns whether the hook was consumed here.
pub(super) fn observe(
    context: &CliContext,
    id: &str,
    runtime_id: &str,
    raw_event_name: Option<&str>,
    raw: &Value,
) -> Result<bool, CliError> {
    if !matches!(
        raw_event_name,
        Some("SessionStart" | "PostModelSwitch" | "ModelSettings")
    ) || raw.get("agent_id").is_some()
    {
        return Ok(false);
    }
    let Some(provider_session) = raw
        .get("session_id")
        .or_else(|| raw.get("session_key"))
        .and_then(Value::as_str)
    else {
        return Ok(false);
    };
    let session_start = raw_event_name == Some("SessionStart");
    // Readiness is the resumed runtime's only idle signal and takes no record
    // lock, so it settles before best-effort model persistence can contend.
    let ready = if session_start
        && matches!(
            raw.get("source").and_then(Value::as_str),
            Some("startup" | "resume")
        ) {
        observe_session_ready(context, id, runtime_id, provider_session)
    } else {
        Ok(())
    };
    let _ = crate::session_model::observe_claude_hook(
        context,
        id,
        runtime_id,
        provider_session,
        raw,
        session_start,
    );
    ready.map(|()| true)
}

/// Claude reports SessionStart(startup|resume) once its composer accepts
/// input, but no turn event follows a resume without a prompt. Only a fresh
/// `starting` projection of this exact runtime and bound conversation becomes
/// provider-reported idle; any turn or attention evidence already wins.
fn observe_session_ready(
    context: &CliContext,
    id: &str,
    runtime_id: &str,
    provider_session: &str,
) -> Result<(), CliError> {
    let record = load_session_record(context, id)?;
    let Some(runtime) = record
        .runtime
        .as_ref()
        .filter(|runtime| runtime.launch_id == runtime_id)
    else {
        return Ok(());
    };
    // agent-hook forwards a runtime-scoped projection, not the raw id.
    if record.agent != AgentKind::Claude.as_str()
        || !crate::session_model::provider_session_matches(&record, provider_session)
    {
        return Ok(());
    }
    let dir = session_dir(context, id);
    let _lock = acquire_lock(&dir)?;
    if !matches!(
        runtime_unhealthy_marker(&dir, runtime_id, runtime.generation),
        RuntimeUnhealthyStatus::Absent
    ) {
        return Ok(());
    }
    let path = dir.join(ACTIVITY_FILE);
    let Ok(mut document) = read_document(&path) else {
        return Ok(());
    };
    if document.runtime_id != runtime_id
        || document.runtime_generation != runtime.generation
        || !replay_matches_document(&dir, &document)
        || document.state.phase != TurnPhase::Starting
        || document.state.current_turn.is_some()
        || !document.pending_attention.is_empty()
        || document.overflow_attention.is_some()
        || document.pending_journal.is_some()
        || document.pending_completion.is_some()
    {
        return Ok(());
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
    write_document(&path, &mut document)
}
