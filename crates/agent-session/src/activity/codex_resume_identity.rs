//! Exact Codex prompt-hook identity for the managed resume record.
use super::*;
use crate::{
    ProviderResume, canonical_provider_resume_args, codex_app_server, mutate_session_record,
};

pub(crate) fn ingest_provider_hook_fail_open(
    context: &CliContext,
    agent: AgentKind,
    event_override: Option<&str>,
) {
    match ingest_provider_hook(context, agent, event_override) {
        Ok(true) => complete_file_hook(context, agent, event_override),
        Ok(false) => {}
        Err(err) => record_hook_diagnostic(context, agent, err.code()),
    }
}

pub(super) fn ingest_dispatched(
    context: &CliContext,
    agent: AgentKind,
    event_override: Option<&str>,
    input: &ProviderHookInput<'_>,
    raw: &Value,
) -> Result<bool, CliError> {
    if agent != AgentKind::Codex || event_override != Some("CodexResumeIdentity") {
        return Ok(false);
    }
    provider_resume_from_user_prompt_hook(
        context,
        input.id,
        agent,
        input.runtime_id,
        event_override,
        raw,
    )?;
    Ok(true)
}

fn complete_file_hook(context: &CliContext, agent: AgentKind, event_override: Option<&str>) {
    clear_hook_diagnostic(context, agent);
    if agent == AgentKind::Codex && event_override == Some("CodexResumeIdentity") {
        let _ = writeln!(io::stdout().lock(), "codex-resume-identity-accepted");
    }
}

pub(super) fn provider_resume_from_user_prompt_hook(
    context: &CliContext,
    id: &str,
    agent: AgentKind,
    runtime_id: &str,
    event_override: Option<&str>,
    raw: &Value,
) -> Result<(), CliError> {
    let event_name = event_override
        .or_else(|| raw.get("hook_event_name").and_then(Value::as_str))
        .or_else(|| raw.get("event").and_then(Value::as_str));
    let dispatched = event_name == Some("CodexResumeIdentity");
    if agent != AgentKind::Codex
        || !matches!(event_name, Some("UserPromptSubmit" | "CodexResumeIdentity"))
    {
        return Ok(());
    }
    let Some(session_id) = raw
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        if dispatched {
            return Err(CliError::data(
                "provider-hook-session-invalid",
                "Codex resume receipt is missing the exact session identity",
                None,
            ));
        }
        return Ok(());
    };
    let projected = dispatched
        .then(|| projected_provider_identifier(runtime_id, agent, "session", session_id))
        .transpose()?;
    let observed = load_session_record(context, id)?;
    let runtime_matches = observed
        .runtime
        .as_ref()
        .is_some_and(|runtime| runtime.launch_id == runtime_id);
    let provider_session_requires_classification = observed
        .provider_resume
        .as_ref()
        .is_none_or(|resume| resume.provider != agent.as_str() || resume.session_id != session_id);
    if runtime_matches
        && provider_session_requires_classification
        && crate::codex_app_server::system_ephemeral_raw_session_is_registered(
            context, &observed, session_id,
        )?
    {
        return Ok(());
    }
    mutate_session_record(context, id, |record| {
        let Some(runtime) = record
            .runtime
            .as_ref()
            .filter(|runtime| runtime.launch_id == runtime_id && record.agent == agent.as_str())
        else {
            return Err(CliError::data(
                "provider-hook-runtime-mismatch",
                "provider hook runtime does not match the active session runtime",
                None,
            ));
        };
        if let Some(projected) = projected.as_deref() {
            // The dispatcher sends this private identity after its projected
            // event is admitted. Bind only to that runtime's observed thread.
            let document = read_document(&session_dir(context, &record.id).join(ACTIVITY_FILE))?;
            if document.runtime_id != runtime_id
                || document.runtime_generation != runtime.generation
                || document.provider_session_id.as_deref() != Some(projected)
            {
                return Err(CliError::data(
                    "provider-hook-session-mismatch",
                    "Codex resume identity does not match the admitted provider session",
                    None,
                ));
            }
        }
        if codex_app_server::app_server_owns_resume_binding(record)
            && (record.provider_resume.is_none()
                || !codex_app_server::thread_attachment_matches(record, session_id))
        {
            return Err(CliError::data(
                "provider-hook-session-mismatch",
                "Codex prompt hook identity requires an established app-server attachment",
                None,
            ));
        }
        if let Some(existing) = record.provider_resume.as_ref() {
            if existing.provider == agent.as_str() && existing.session_id == session_id {
                let resume_args = canonical_provider_resume_args(agent, &record.cwd, session_id)
                    .ok_or_else(|| {
                        CliError::data(
                            "provider-hook-session-invalid",
                            "provider hook session identity cannot be resumed",
                            None,
                        )
                    })?;
                if existing.capture_method == "codex-user-prompt-submit-hook"
                    && existing.resume_args == resume_args
                {
                    return Ok(());
                }
                let mut promoted = existing.clone();
                promoted.captured_at = Zoned::now().timestamp().to_string();
                promoted.capture_method = "codex-user-prompt-submit-hook".to_string();
                promoted.resume_args = resume_args;
                record.provider_resume = Some(promoted);
                return Ok(());
            }
            return Err(CliError::data(
                "provider-hook-session-mismatch",
                "provider hook session identity conflicts with the durable session identity",
                None,
            ));
        }
        let resume_args = canonical_provider_resume_args(agent, &record.cwd, session_id)
            .ok_or_else(|| {
                CliError::data(
                    "provider-hook-session-invalid",
                    "provider hook session identity cannot be resumed",
                    None,
                )
            })?;
        record.provider_resume = Some(ProviderResume {
            provider: agent.as_str().to_string(),
            session_id: session_id.to_string(),
            captured_at: Zoned::now().timestamp().to_string(),
            capture_method: "codex-user-prompt-submit-hook".to_string(),
            resume_args,
            extra: BTreeMap::new(),
        });
        Ok(())
    })
}
