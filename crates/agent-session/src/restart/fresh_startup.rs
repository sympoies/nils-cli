//! Fresh recovery is confined to failed Codex startup, never an existing conversation.
use super::*;

pub(crate) fn eligible(context: &CliContext, record: &SessionRecord) -> bool {
    record.mode == "interactive"
        && record.provider_resume.is_none()
        && crate::codex_app_server::runtime_is_supported(record)
        && crate::codex_account::view_for_record(record).state == "failed"
        && crate::codex_account::queued_switch_repair(record).is_some()
        && crate::codex_app_server::thread_attached_path(record).is_some_and(|path| !path.exists())
        && crate::activity::state_for_view(context, record).is_some_and(|state| {
            state.phase == crate::activity::TurnPhase::Starting && state.current_turn.is_none()
        })
}

pub(crate) struct LaunchPlan {
    pub(crate) agent: AgentKind,
    pub(crate) args: Vec<String>,
    pub(crate) conversation: Option<String>,
}

pub(crate) fn conversation_guards(
    context: &CliContext,
    record: &SessionRecord,
    plan: &LaunchPlan,
    tmux: &Path,
) -> Result<(Option<std::fs::File>, Vec<std::fs::File>), CliError> {
    let Some(id) = plan.conversation.as_deref() else {
        return Ok((None, Vec::new()));
    };
    let conversation = crate::start_resume::conversation_lock(context, plan.agent, id)?;
    let sources =
        crate::start_resume::guard_other_writers(context, plan.agent, id, Some(&record.id), tmux)?;
    Ok((Some(conversation), sources))
}

pub(crate) fn launch_plan(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<LaunchPlan, CliError> {
    if eligible(context, record) {
        crate::validate_stored_agent_args(record, AgentKind::Codex)?;
        return Ok(LaunchPlan {
            agent: AgentKind::Codex,
            args: Vec::new(),
            conversation: None,
        });
    }
    let (resume, agent) = crate::validate_resume_metadata(record)?;
    Ok(LaunchPlan {
        agent,
        args: resume.resume_args.clone(),
        conversation: Some(resume.session_id.clone()),
    })
}

pub(crate) fn release_bootstrap(
    bootstrap: Option<crate::codex_app_server::CreateBootstrapGuard>,
    lock: crate::SessionRecordLock,
) {
    if let Some(bootstrap) = bootstrap {
        bootstrap.finish(|| drop(lock));
    } else {
        drop(lock);
    }
}
