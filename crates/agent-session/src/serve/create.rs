//! Ordinary HTTP creation, shared by fresh and account-bound resumed launches.
use super::*;

pub(super) async fn create_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    if body.resume.is_some() || body.resume_account_explicit.is_some() {
        return envelope_err(crate::start_resume::refusal("resume-unsupported"));
    }
    if let Err(error) = crate::launch_env::validate(&body.launch_env) {
        return envelope_err(error);
    }
    let origin = match body.origin_fields() {
        Ok(origin) => origin,
        Err(error) => return envelope_err(error),
    };
    let operation = body.operation();
    let id = match crate::lifecycle::start_id(
        &state.context,
        body.id.as_deref(),
        AgentKind::from_name(&body.agent).unwrap_or(AgentKind::Codex),
        match &body.title {
            CreateTitleInput::Value(title) => Some(title.as_str()),
            _ => None,
        },
    ) {
        Ok(id) => id,
        Err(error) => return envelope_err(error),
    };
    let context = state.context.clone();
    let before = load_session_record(&context, &id).ok();
    let response = create_handler_inner(State(state), headers, Json(body), origin).await;
    crate::lifecycle::response(&context, &id, operation, before, response).await
}

pub(super) async fn create_handler_inner(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
    origin: crate::origin::Fields,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let Some(agent) = AgentKind::from_name(&body.agent) else {
        return envelope_err(CliError::usage(
            "invalid-agent",
            format!("unknown agent: {}", body.agent),
            None,
        ));
    };
    if agent == AgentKind::Dsh {
        return envelope_err(CliError::usage(
            "unsupported-agent",
            "DSH sessions are no longer supported; create a session with a supported agent",
            Some(json!({ "agent": body.agent })),
        ));
    }
    let resuming = body.resume.is_some();
    if resuming
        && (body.agent_profile.is_some()
            || body.provider_resume_id.is_some()
            || body.codex_account.is_some())
    {
        return envelope_err(crate::start_resume::refusal("resume-unsupported"));
    }
    let launch_profile = match body.agent_profile.as_deref() {
        Some(id) => {
            let Some(profile) = state.launch_profiles.get(id) else {
                return envelope_err(CliError::usage(
                    "unknown-agent-profile",
                    "the requested agent profile is not configured on this machine",
                    Some(json!({ "agent_profile": id })),
                ));
            };
            if profile.agent != agent {
                return envelope_err(CliError::usage(
                    "agent-profile-agent-conflict",
                    "the requested agent profile does not match the base agent",
                    Some(json!({
                        "agent": agent.as_str(),
                        "agent_profile": id,
                    })),
                ));
            }
            let profile = profile.clone();
            let readiness_profile = profile.clone();
            let ready =
                match tokio::task::spawn_blocking(move || readiness_profile.is_ready()).await {
                    Ok(ready) => ready,
                    Err(_) => return join_err(),
                };
            if !ready {
                return envelope_err(CliError::runtime(
                    "agent-profile-unavailable",
                    "the requested agent profile is not ready on this machine",
                    Some(json!({ "agent_profile": id })),
                ));
            }
            Some(profile)
        }
        None => None,
    };
    let context = state.context.clone();
    let title_supplied = body.title.is_supplied();
    let title = body.title.into_value();
    let title_state = body.title_state.map(SessionTitleState::from);
    let (title, title_state) = match title_state {
        Some(title_state) => {
            match canonicalize_structured_title_pair(title, title_supplied, title_state) {
                Ok(pair) => pair,
                Err(err) => return envelope_err(err),
            }
        }
        None => (title, None),
    };
    if let Err(err) = validate_create_accounts(
        agent,
        body.codex_account.is_some(),
        body.claude_account.is_some(),
        launch_profile.is_some(),
        body.provider_resume_id.is_some(),
    ) {
        return envelope_err(err);
    }
    let mut initial_lineage = match create_body_lineage(
        &state.machine,
        body.lineage.as_ref(),
        body.work.as_ref(),
        body.role.as_ref(),
    ) {
        Ok(initial_lineage) => initial_lineage,
        Err(err) => return envelope_err(err),
    };
    initial_lineage.origin = origin;
    if let Some(provider_resume_id) = body.provider_resume_id {
        if body.cwd.is_some() {
            return envelope_err(CliError::usage(
                "provider-resume-cwd-conflict",
                "provider_resume_id mode resolves cwd from provider history; omit cwd",
                None,
            ));
        }
        if body.prompt.is_some() {
            return envelope_err(CliError::usage(
                "provider-resume-prompt-conflict",
                "provider_resume_id mode imports an existing provider session; omit prompt",
                None,
            ));
        }
        if launch_profile
            .as_ref()
            .is_some_and(|profile| profile.provider_config_dir.is_none())
        {
            return envelope_err(CliError::runtime(
                "agent-profile-provider-root-unavailable",
                "the requested agent profile does not declare a provider configuration root for resume import",
                launch_profile
                    .as_ref()
                    .map(|profile| json!({ "agent_profile": profile.id })),
            ));
        }
        let args = ProviderResumeImportArgs {
            ordinary_start: None,
            launch_env: body.launch_env,
            title_mode: body.title_mode,
            agent,
            provider_resume_id,
            title,
            title_state,
            id: body.id,
            coordination_mode: body.coordination_mode,
            tmux_bin: Some(state.tmux_bin.clone()),
            agent_bin: launch_profile
                .as_ref()
                .map(|profile| profile.agent_bin.clone()),
            agent_profile: launch_profile.as_ref().map(|profile| profile.id.clone()),
            provider_config_dir: launch_profile
                .as_ref()
                .and_then(|profile| profile.provider_config_dir.clone()),
            profile_auto_resume_supported: launch_profile
                .as_ref()
                .map(|profile| profile.auto_resume_supported),
            profile_graceful_shutdown: launch_profile
                .as_ref()
                .and_then(|profile| profile.graceful_shutdown)
                .map(|mode| mode.as_str().to_string()),
            codex_usage_account: launch_profile
                .as_ref()
                .and_then(|profile| profile.codex_usage_account.clone()),
            agent_args: body.agent_args,
            initial_lineage,
            format: nils_common::cli_contract::OutputFormat::Json,
        };
        return match tokio::task::spawn_blocking(move || {
            let _journal_owner = crate::lifecycle::ServeGuard::enter("create");
            start_provider_resume_session(&context, args)
        })
        .await
        {
            Ok(Ok(view)) => {
                envelope_ok(json!({ "machine": state.machine, "session": view.result }))
            }
            Ok(Err(err)) => envelope_err(err),
            Err(_) => join_err(),
        };
    }
    let explicit_account = body.codex_account;
    let (selected_account, selection_source) = if resuming {
        (None, None)
    } else {
        match tokio::task::spawn_blocking(move || {
            let _journal_owner = crate::lifecycle::ServeGuard::enter("create");
            resolve_initial_codex_account(agent, explicit_account)
        })
        .await
        {
            Ok(Ok(selection)) => selection,
            Ok(Err(error)) => return envelope_err(error),
            Err(_) => return join_err(),
        }
    };
    // A launch profile owns its provider root, so only a profile-free Claude
    // create binds an account (explicitly or from the broker's default).
    let claude_account = if !resuming && agent == AgentKind::Claude && launch_profile.is_none() {
        let explicit_claude_account = body.claude_account.clone();
        match tokio::task::spawn_blocking(move || {
            let _journal_owner = crate::lifecycle::ServeGuard::enter("create");
            crate::claude_account::resolve_initial_account(explicit_claude_account)
        })
        .await
        {
            Ok(Ok(selection)) => {
                selection.map(|(account, selection_source)| crate::InitialClaudeAccount {
                    account,
                    selection_source,
                })
            }
            Ok(Err(error)) => return envelope_err(error),
            Err(_) => return join_err(),
        }
    } else {
        None
    };
    let prompt = body.prompt;
    let deferred_prompt = selected_account.as_ref().and_then(|_| prompt.clone());
    let args = cli::StartArgs {
        resume: body.resume,
        check_resume: false,
        initial_resume_account_explicit: body.resume_account_explicit,
        workspace: None,
        title_mode: match body.title_mode {
            crate::display_metadata::TitleMode::Auto => "auto",
            crate::display_metadata::TitleMode::Pinned => "pinned",
        }
        .into(),
        app_server_managed: true,
        initial_codex_account: selected_account.clone(),
        initial_codex_account_source: selection_source,
        initial_title_state: title_state,
        initial_agent_profile: launch_profile.as_ref().map(|profile| profile.id.clone()),
        initial_provider_config_dir: launch_profile
            .as_ref()
            .and_then(|profile| profile.provider_config_dir.clone()),
        initial_profile_auto_resume_supported: launch_profile
            .as_ref()
            .map(|profile| profile.auto_resume_supported),
        initial_profile_graceful_shutdown: launch_profile
            .as_ref()
            .and_then(|profile| profile.graceful_shutdown)
            .map(|mode| mode.as_str().to_string()),
        initial_codex_usage_account: launch_profile
            .as_ref()
            .and_then(|profile| profile.codex_usage_account.clone()),
        initial_lineage: Some(initial_lineage),
        agent,
        cwd: body.cwd.map(PathBuf::from),
        title,
        id: body.id,
        prompt: if selected_account.is_some() {
            None
        } else {
            prompt
        },
        prompt_file: None,
        prompt_stdin: false,
        notice: Default::default(),
        tmux_bin: Some(state.tmux_bin.clone()),
        agent_bin: launch_profile
            .as_ref()
            .map(|profile| profile.agent_bin.clone()),
        agent_args: body.agent_args,
        coordination_mode: body.coordination_mode,
        launch_env: body
            .launch_env
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect(),
        paste_delay_ms: cli::DEFAULT_PASTE_DELAY_MS,
        via_console: false,
        console_operation: Default::default(),
        machine: None,
        account: if resuming { body.claude_account } else { None },
        agent_profile: None,
        no_parent: false,
        forge_initiator: None,
        role: None,
        program: None,
        issues: Vec::new(),
        no_inherit_work: false,
        format: nils_common::cli_contract::OutputFormat::Json,
    };
    match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("create");
        crate::start_session_with_claude_account(
            &context,
            args,
            StartFailureDisposition::ReturnError,
            claude_account,
        )
    })
    .await
    {
        Ok(Ok(mut view)) => {
            if selected_account.is_some() {
                let Some(launch_id) = view
                    .result
                    .session_incarnation
                    .as_deref()
                    .map(str::to_string)
                else {
                    return status_json(
                        StatusCode::CONFLICT,
                        "codex-account-unsupported",
                        "created Codex session does not support account binding",
                    );
                };
                let requested_account = selected_account.as_deref().unwrap_or_default();
                let binding = match wait_for_account_binding(
                    &state,
                    &view.result.id,
                    &launch_id,
                    requested_account,
                    1,
                )
                .await
                {
                    Ok(binding) => binding,
                    Err(response) => return response,
                };
                view.result.codex_account = binding;
                if let Some(prompt) = deferred_prompt {
                    let Some(handle) = wait_for_codex_control_within(
                        &state,
                        &view.result.id,
                        &launch_id,
                        CODEX_CREATE_PROMPT_CONTROL_WAIT,
                    )
                    .await
                    else {
                        return status_json(
                            StatusCode::CONFLICT,
                            "structured-prompt-unavailable",
                            "structured prompt control is not ready for this session",
                        );
                    };
                    if let Err(response) = submit_structured_prompt_locked(
                        &state,
                        &view.result.id,
                        &launch_id,
                        StructuredPromptGuards {
                            expected_binding: Some((requested_account, 1)),
                            ..Default::default()
                        },
                        &handle,
                        &prompt,
                    )
                    .await
                    {
                        return response;
                    }
                }
            }
            envelope_ok(json!({ "machine": state.machine, "session": view.result }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}
