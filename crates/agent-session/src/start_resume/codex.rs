//! Codex uses its retained broker binding and canonical local rollout history.
use super::*;

pub(super) fn admit(
    context: &CliContext,
    request: &Request,
    tmux: &Path,
    agent_bin: Option<&Path>,
) -> Result<Admission, CliError> {
    let (records, sources) = matching_sources(context, AgentKind::Codex, &request.resume, None)?;
    if records.is_empty() {
        resolve_provider_resume_source(AgentKind::Codex, &request.resume, None, None)
            .map_err(history_error)?;
        // Shared host history and an execution-account pin do not prove the
        // original broker account after every managed source was deleted.
        return Err(refusal("resume-source-unknown"));
    }
    let mut original = None;
    for record in &records {
        let codex_account::BindingSnapshot::Bound { account, .. } =
            codex_account::binding_snapshot(record)
        else {
            return Err(refusal("resume-source-unknown"));
        };
        if !matches!(
            codex_account::next_transition_state(record),
            codex_account::NextTransitionState::Absent
        ) {
            return Err(refusal("resume-source-unknown"));
        }
        if request
            .account
            .as_ref()
            .is_some_and(|requested| requested != &account)
        {
            return Err(refusal("resume-account-mismatch"));
        }
        let root = session_provider_config_dir(record)
            .or_else(|| {
                nils_provider_resume::codex_sessions_root()
                    .and_then(|root| root.parent().map(Path::to_path_buf))
            })
            .ok_or_else(|| refusal("resume-history-missing"))?;
        let root = fs::canonicalize(root).map_err(|_| refusal("resume-history-missing"))?;
        let binding = (account, root);
        if original
            .as_ref()
            .is_some_and(|original| original != &binding)
        {
            return Err(refusal(if request.account.is_some() {
                "resume-account-mismatch"
            } else {
                "resume-source-unknown"
            }));
        }
        original = Some(binding);
        let mut requested_launch = record.clone();
        requested_launch.agent_args = request.agent_args.clone();
        if !runtime_environment::provider_facts::account_applicable(&requested_launch) {
            return Err(refusal("resume-unsupported"));
        }
    }
    let (account, config_dir) = original.expect("retained Codex source");
    codex_account::validate_account(&account).map_err(|_| refusal("resume-source-unknown"))?;
    let accounts = codex_account::list_accounts().map_err(|_| refusal("resume-unsupported"))?;
    if !accounts.iter().any(|entry| entry.account == account) {
        return Err(refusal("resume-account-mismatch"));
    }
    let source =
        resolve_provider_resume_source(AgentKind::Codex, &request.resume, Some(&config_dir), None)
            .map_err(history_error)?;
    let cwd = resolve_cwd(Some(&source.cwd)).map_err(|_| refusal("resume-unsupported"))?;
    if let Some(requested) = request.cwd.as_deref()
        && resolve_cwd(Some(requested)).map_err(|_| refusal("resume-unsupported"))? != cwd
    {
        return Err(refusal("resume-unsupported"));
    }
    for record in &records {
        if resolve_cwd(Some(Path::new(&record.cwd)))
            .map_err(|_| refusal("resume-source-unknown"))?
            != cwd
        {
            return Err(refusal("resume-source-unknown"));
        }
        prove_stopped(context, record, tmux)?;
    }
    if !codex_app_server::resume_transport_supported(&resolve_agent_bin(
        AgentKind::Codex,
        agent_bin,
    )) {
        return Err(refusal("resume-unsupported"));
    }
    Ok(Admission {
        account,
        config_dir,
        cwd,
        history_scope: None,
        abandoned_sources: Vec::new(),
        _sources: sources,
    })
}

/// Start may prepare the ordinary private socket root. Checks never call this.
pub(super) fn prepare(args: &cli::StartArgs, context: &CliContext) -> Result<(), CliError> {
    if !codex_app_server::resume_runtime_ready(
        &resolve_agent_bin(AgentKind::Codex, args.agent_bin.as_deref()),
        &context.state_dir,
    ) {
        return Err(refusal("resume-unsupported"));
    }
    Ok(())
}
