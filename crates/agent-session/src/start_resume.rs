//! New managed records from an exact, account-bound provider conversation.
use super::*;

pub(super) fn run_start(context: &CliContext, mut args: cli::StartArgs) -> i32 {
    let format = args.format;
    if args.check_resume {
        return run_check(context, &args);
    }
    if args.account.is_some() && args.resume.is_none() && !args.via_console {
        return render_error(START_COMMAND, format, refusal("resume-unsupported"));
    }
    if let Err(error) = notice::prepare_start(&mut args) {
        return render_error(START_COMMAND, format, error);
    }
    if let Err(error) = launch_env::from_flags(&args.launch_env) {
        return render_error(START_COMMAND, format, error);
    }
    if args.forge_initiator.is_some() && non_empty_env("AGENT_SESSION_ID").is_some() {
        return render_error(
            START_COMMAND,
            format,
            CliError::data(
                "forge-initiator-rebind-forbidden",
                "managed sessions cannot replace their launch initiator",
                None,
            ),
        );
    }
    if let Err(err) = lineage::role_from_request(args.role.as_deref()) {
        return render_error(START_COMMAND, format, err);
    }
    let work = match lineage::WorkRequest::from_flags(
        args.program.as_deref(),
        &args.issues,
        args.no_inherit_work,
    ) {
        Ok(work) => work,
        Err(err) => {
            let command = if args.via_console {
                CONSOLE_START_COMMAND
            } else {
                START_COMMAND
            };
            return render_error(command, format, err);
        }
    };
    if args.via_console {
        if let Err(err) = lineage::require_root_for_role(args.role.as_deref(), !args.no_parent) {
            return render_error(CONSOLE_START_COMMAND, format, err);
        }
        let command = if args.resume.is_some() {
            START_COMMAND
        } else {
            CONSOLE_START_COMMAND
        };
        return match start_via_console(context, args, &work) {
            Ok(result) => {
                render_single_success(command, format, &result, render_console_started_text)
            }
            Err(err) => render_error(command, format, err),
        };
    }
    let (mut initial_lineage, warning) =
        match lineage::resolve_cli_start(context, args.no_parent, args.role.as_deref(), &work) {
            Ok(resolved) => resolved,
            Err(err) => return render_error(START_COMMAND, format, err),
        };
    if let Some(warning) = warning {
        eprintln!("warning: {warning}");
    }
    if let Err(err) =
        lineage::require_root_for_role(args.role.as_deref(), initial_lineage.seed.has_parent())
    {
        return render_error(START_COMMAND, format, err);
    }
    if let Some(initiator) = &args.forge_initiator {
        let context = nils_common::forge_identity::session::LaunchContext {
            initiator: initiator.clone(),
            role: args.role.clone(),
        };
        if let Err(err) = context.validate() {
            return render_error(START_COMMAND, format, forge_identity::error(err));
        }
        initial_lineage.seed.set_forge_context(Some(context));
    }
    args.initial_lineage = Some(crate::InitialLineage {
        role: args.role.clone(),
        ..initial_lineage
    });
    let resuming = args.resume.is_some();
    match start_session(context, args, StartFailureDisposition::ReturnError) {
        Ok(view) if resuming => render_single_success(
            START_COMMAND,
            view.format,
            &json!({"session":view.result}),
            |value| format!("started resumed session {}\n", value["session"]["id"]),
        ),
        Ok(view) => render_single_success(
            START_COMMAND,
            view.format,
            &view.result,
            render_started_text,
        ),
        Err(err) => render_error(START_COMMAND, format, err),
    }
}

const CHECK_COMMAND: &str = "resume-preflight";

pub(super) fn refusal(code: &'static str) -> CliError {
    let message = match code {
        "resume-history-missing" => {
            "the exact provider conversation is unavailable in the selected account history"
        }
        "resume-account-mismatch" => {
            "the requested account does not match the original conversation account"
        }
        "resume-source-live" => "a managed source conversation still has a live or busy writer",
        "resume-source-unknown" => {
            "source account, ownership or stopped-runtime evidence is incomplete"
        }
        _ => "this resume request is not supported",
    };
    CliError::data(
        code,
        message,
        Some(json!({"session_created":false,"safe_to_retry":false})),
    )
}

struct Admission {
    account: String,
    config_dir: PathBuf,
    cwd: PathBuf,
    history_scope: claude_account::history::HistoryScope,
    abandoned_sources: Vec<SessionRecord>,
    // Existing source locks are observational: opening/flocking never creates
    // or rewrites a source record. Keep them until the new launch is committed.
    _sources: Vec<fs::File>,
}

/// The deliberately small, prompt-free read-only transport contract.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub agent: String,
    pub resume: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "claude_account"
    )]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_args: Vec<String>,
}

impl Request {
    pub(crate) fn from_args(args: &cli::StartArgs) -> Self {
        Self {
            agent: args.agent.as_str().into(),
            resume: args.resume.clone().unwrap_or_default(),
            cwd: args.cwd.clone(),
            account: args.account.clone(),
            agent_profile: args
                .agent_profile
                .clone()
                .or(args.initial_agent_profile.clone()),
            agent_args: args.agent_args.clone(),
        }
    }
    pub(crate) fn validate(&self) -> Result<AgentKind, CliError> {
        let agent =
            AgentKind::from_name(&self.agent).ok_or_else(|| refusal("resume-unsupported"))?;
        if self.agent_profile.is_some()
            || agent != AgentKind::Claude
            || uuid::Uuid::parse_str(&self.resume).is_err()
            || uuid::Uuid::parse_str(&self.resume).is_ok_and(|id| id.to_string() != self.resume)
        {
            return Err(refusal("resume-unsupported"));
        }
        validate_agent_args(agent, &self.agent_args).map_err(|_| refusal("resume-unsupported"))?;
        Ok(agent)
    }
}

fn lock_source(context: &CliContext, id: &str) -> Result<fs::File, CliError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(
            context
                .state_dir
                .join(SESSION_LOCKS_DIR)
                .join(format!("{id}.lock")),
        )
        .map_err(|_| refusal("resume-source-unknown"))?;
    // SAFETY: the descriptor remains owned by Admission for the whole launch.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(refusal(
            if io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
                "resume-source-live"
            } else {
                "resume-source-unknown"
            },
        ));
    }
    Ok(file)
}

fn source_account(
    record: &SessionRecord,
    requested: Option<&str>,
) -> Result<(String, PathBuf), CliError> {
    let unknown = || refusal("resume-source-unknown");
    let mut bound = record.clone();
    let pending = claude_account::abandon_next_for_resume(&mut bound).map_err(|_| unknown())?;
    let view = claude_account::view_for_record(&bound).ok_or_else(unknown)?;
    if view.state != "bound" {
        return Err(unknown());
    }
    let account = view.selected_account.ok_or_else(unknown)?;
    if pending && requested.is_none() {
        return Err(CliError::data(
            "resume-account-transition-pending",
            "select the bound account explicitly to abandon the pending switch, or resume the existing session to apply it",
            Some(json!({"session_created":false,"safe_to_retry":false})),
        ).with_hint(format!("run `agent-session resume {}` to retry the queued account", record.id)));
    }
    Ok((
        account,
        claude_account::config_dir_for_runtime(&bound).ok_or_else(unknown)?,
    ))
}

fn prove_stopped(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
) -> Result<(), CliError> {
    match restart::resume_status(tmux, record) {
        restart::ResumeStatus::Running => return Err(refusal("resume-source-live")),
        restart::ResumeStatus::Unknown { .. } => return Err(refusal("resume-source-unknown")),
        restart::ResumeStatus::Stopped => {}
    }
    let unknown = || refusal("resume-source-unknown");
    let prior = persisted_prior_tmux_runtime_identities(record).map_err(|_| unknown())?;
    verify_stopped_process_runtimes(&prior, Duration::ZERO).map_err(|_| unknown())?;
    match persisted_tmux_runtime_identity(record).map_err(|_| unknown())? {
        Some(identity) => {
            verify_stopped_tmux_runtime(tmux, &record.tmux_session, &identity, Duration::ZERO)
                .map_err(|_| unknown())?
        }
        None if runtime_is_proven_never_launched(record) && prior.is_empty() => {}
        None => return Err(unknown()),
    }
    if matches!(
        persisted_tmux_termination_state(record).map_err(|_| unknown())?,
        Some(TmuxTerminationState::Pending { .. } | TmuxTerminationState::FreezePending { .. })
    ) {
        return Err(unknown());
    }
    // A new record is not an escape from auth quarantine or an unsettled
    // provider/account transition. Existing recovery owns those decisions.
    coordination::auth_effects::resume_source_closed(context, record)?;
    auth_incident::recovery::ensure_input_allowed(context, record).map_err(|_| unknown())?;
    Ok(())
}

fn matching_sources(
    context: &CliContext,
    agent: AgentKind,
    id: &str,
    exclude: Option<&str>,
) -> Result<(Vec<SessionRecord>, Vec<fs::File>), CliError> {
    let mut matching = Vec::new();
    let mut sources = Vec::new();
    let entries = match fs::read_dir(context.state_dir.join("sessions")) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return Err(refusal("resume-source-unknown")),
    };
    for entry in entries.into_iter().flatten() {
        let entry = entry.map_err(|_| refusal("resume-source-unknown"))?;
        let kind = entry
            .file_type()
            .map_err(|_| refusal("resume-source-unknown"))?;
        if kind.is_symlink() {
            return Err(refusal("resume-source-unknown"));
        }
        if !kind.is_dir() {
            continue;
        }
        let source_id = entry
            .file_name()
            .into_string()
            .map_err(|_| refusal("resume-source-unknown"))?;
        if exclude == Some(source_id.as_str()) {
            continue;
        }
        let read = || {
            let path = entry.path().join("session.json");
            let resolved = ensure_record_in_session_dir(context, &path, &entry.path(), &source_id)
                .map_err(|_| refusal("resume-source-unknown"))?;
            let record = read_session_record(&resolved.record_path)
                .map_err(|_| refusal("resume-source-unknown"))?;
            validate_record_id(&record, &source_id, &resolved.record_path)
                .map_err(|_| refusal("resume-source-unknown"))?;
            Ok::<_, CliError>(record)
        };
        let observed = read()?;
        let matches = |r: &SessionRecord| {
            r.agent == agent.as_str()
                && r.provider_resume
                    .as_ref()
                    .is_some_and(|p| p.provider == agent.as_str() && p.session_id == id)
        };
        if !matches(&observed) {
            continue;
        }
        sources.push(lock_source(context, &source_id)?);
        let current = read()?;
        if !matches(&current) {
            return Err(refusal("resume-source-unknown"));
        }
        matching.push(current);
    }
    Ok((matching, sources))
}

pub(super) fn guard_other_writers(
    context: &CliContext,
    agent: AgentKind,
    id: &str,
    exclude: Option<&str>,
    tmux: &Path,
) -> Result<Vec<fs::File>, CliError> {
    let (records, locks) = matching_sources(context, agent, id, exclude)?;
    for record in &records {
        prove_stopped(context, record, tmux)?;
    }
    Ok(locks)
}

fn admit(
    context: &CliContext,
    args: &Request,
    tmux: &Path,
    account_explicit: bool,
) -> Result<Admission, CliError> {
    let agent = args.validate()?;
    let id = &args.resume;
    let (matching, sources) = matching_sources(context, agent, id, None)?;
    let mut history_scope = None;
    let mut original = if matching.is_empty() {
        // An execution-account pin is not original deleted-source authority.
        match args.account.as_deref().filter(|_| account_explicit) {
            Some(account) => {
                let resolved = claude_account::history::root(account)?;
                history_scope = Some(resolved.history_scope);
                let config_dir = resolved.config_dir;
                // The shared resolver deduplicates by cwd. Deleted-source
                // admission needs one exact file, even when copies share cwd.
                let mut matches = std::collections::BTreeSet::new();
                let mut budget = nils_provider_resume::ClaudeResumeScanBudget::from_env();
                nils_provider_resume::collect_claude_provider_resume_matches(
                    &config_dir.join("projects"),
                    id,
                    &mut matches,
                    &mut budget,
                );
                if budget.truncated || matches.len() > 1 {
                    return Err(refusal("resume-source-unknown"));
                }
                if !matches.iter().any(|candidate| {
                    candidate.path.file_name().and_then(|name| name.to_str())
                        == Some(format!("{id}.jsonl").as_str())
                }) {
                    return Err(refusal("resume-history-missing"));
                }
                Some((account.to_string(), config_dir))
            }
            None => {
                // Host-login history alone is never account authority.
                resolve_provider_resume_source(agent, id, None, None).map_err(history_error)?;
                return Err(refusal("resume-source-unknown"));
            }
        }
    } else {
        None
    };
    for record in &matching {
        let account = source_account(record, args.account.as_deref().filter(|_| account_explicit))?;
        if args.account.as_ref().is_some_and(|a| a != &account.0) {
            return Err(refusal("resume-account-mismatch"));
        }
        if original.as_ref().is_some_and(|a| a != &account) {
            return Err(refusal(if args.account.is_some() {
                "resume-account-mismatch"
            } else {
                "resume-source-unknown"
            }));
        }
        original = Some(account);
    }
    let (account, config_dir) = original.expect("admitted account authority");
    if account.is_empty() {
        return Err(refusal("resume-source-unknown"));
    }
    let source = resolve_provider_resume_source(agent, id, Some(&config_dir), None)
        .map_err(history_error)?;
    let cwd = resolve_cwd(Some(&source.cwd)).map_err(|_| refusal("resume-unsupported"))?;
    if let Some(requested) = args.cwd.as_deref() {
        let requested = resolve_cwd(Some(requested)).map_err(|_| refusal("resume-unsupported"))?;
        if requested != cwd {
            return Err(refusal("resume-unsupported"));
        }
    }
    claude_account::validate_config_dir(&config_dir, unsafe { libc::geteuid() })
        .map_err(|_| refusal("resume-account-mismatch"))?;
    // Deleted-source root resolution already checked this roster.
    if !matching.is_empty() {
        let accounts = claude_account::list_accounts()
            .map_err(|_| refusal("resume-unsupported"))?
            .accounts;
        if !accounts.iter().any(|a| a.account == account) {
            return Err(refusal("resume-account-mismatch"));
        }
    }
    for record in &matching {
        prove_stopped(context, record, tmux)?;
    }
    let history_scope = match history_scope {
        Some(scope) => scope,
        None => claude_account::history::observed_scope(&config_dir)?,
    };
    let mut abandoned_sources = Vec::new();
    for mut record in matching {
        if claude_account::abandon_next_for_resume(&mut record)
            .map_err(|_| refusal("resume-source-unknown"))?
        {
            abandoned_sources.push(record);
        }
    }
    Ok(Admission {
        account,
        config_dir,
        cwd,
        history_scope,
        abandoned_sources,
        _sources: sources,
    })
}

pub(super) fn history_error(error: CliError) -> CliError {
    refusal(match error.code() {
        "provider-resume-not-found" => "resume-history-missing",
        "provider-resume-ambiguous" | "provider-resume-scan-truncated" => "resume-source-unknown",
        _ => "resume-history-missing",
    })
}

pub(crate) const PREFLIGHT_SCHEMA: &str = "cli.agent-session.resume-preflight.v1";

pub(crate) fn preflight(
    context: &CliContext,
    request: &Request,
    tmux: &Path,
    machine: &str,
) -> Result<Value, CliError> {
    account_broker::adopt_serve_brokers(&context.state_dir);
    let admitted = admit(context, request, tmux, true)?;
    Ok(
        json!({"agent":request.agent,"provider_session_id":request.resume,"machine":machine,"account":admitted.account,"history_scope":admitted.history_scope}),
    )
}

fn run_check(context: &CliContext, args: &cli::StartArgs) -> i32 {
    let result = if args.via_console {
        coordination::resume_preflight::check_args(context, args)
    } else {
        preflight(
            context,
            &Request::from_args(args),
            &resolve_tmux_bin(args.tmux_bin.as_deref()),
            &board::machine_identity(None, context),
        )
    };
    match result {
        Ok(data) => render_single_success(CHECK_COMMAND, args.format, &data, |value| {
            format!(
                "resume available for {} on {}\n",
                value["agent"], value["machine"]
            )
        }),
        Err(error) => render_error(CHECK_COMMAND, args.format, error),
    }
}

pub(super) fn start(context: &CliContext, mut args: cli::StartArgs) -> Result<StartView, CliError> {
    account_broker::adopt_serve_brokers(&context.state_dir);
    // An observational pass happens before any lock/store/operation creation.
    // It is not authorization: guarded admission is repeated after preparation
    // and holds every final source lock through ordinary persistence/launch.
    let mut request = Request::from_args(&args);
    let tmux = resolve_tmux_bin(args.tmux_bin.as_deref());
    let account_explicit = args.initial_resume_account_explicit.unwrap_or(true);
    drop(admit(context, &request, &tmux, account_explicit)?);
    let _conversation = conversation_lock(context, args.agent, args.resume.as_deref().unwrap())
        .map_err(|error| {
            refusal(if error.code() == "coordination-lock-timeout" {
                "resume-source-live"
            } else {
                "resume-source-unknown"
            })
        })?;
    let admitted = admit(context, &request, &tmux, account_explicit)?;
    let selected_account = admitted.account.clone();
    args.cwd = Some(admitted.cwd.clone());
    request.cwd = args.cwd.clone();
    let prepared = claude_account::materialize(&selected_account)
        .map_err(|_| refusal("resume-account-mismatch"))?;
    if prepared != admitted.config_dir {
        return Err(refusal("resume-account-mismatch"));
    }
    // Broker preparation is an external call. Keep the conversation guard,
    // then reacquire all source guards and recheck authority/history so a
    // source first observed during preparation cannot bypass final admission.
    drop(admitted);
    // Keep the original account request here: a resolved account is not
    // explicit consent to cancel an intent that appeared during preparation.
    let admitted = admit(context, &request, &tmux, account_explicit)?;
    if admitted.account != selected_account || prepared != admitted.config_dir {
        return Err(refusal("resume-account-mismatch"));
    }
    args.account = Some(admitted.account.clone());
    // Settle only the final admitted source snapshots while their writer
    // guards and the conversation guard remain held. Failed final admission
    // must leave any pending account transition untouched.
    for mut source in admitted.abandoned_sources {
        source.updated_at = jiff::Timestamp::now().to_string();
        write_session_record(context, &source)?;
    }
    args.initial_provider_config_dir = Some(prepared);
    let initial_lineage = args
        .initial_lineage
        .take()
        .ok_or_else(|| refusal("resume-source-unknown"))?;
    let import = ProviderResumeImportArgs {
        launch_env: launch_env::from_flags(&args.launch_env)?,
        agent: args.agent,
        title_mode: if args.title_mode == "pinned" {
            display_metadata::TitleMode::Pinned
        } else {
            display_metadata::TitleMode::Auto
        },
        provider_resume_id: args.resume.clone().unwrap(),
        title: args.title.clone(),
        title_state: args.initial_title_state.clone(),
        id: args.id.clone(),
        coordination_mode: args.coordination_mode,
        tmux_bin: args.tmux_bin.clone(),
        agent_bin: args.agent_bin.clone(),
        agent_profile: None,
        provider_config_dir: Some(admitted.config_dir.clone()),
        profile_auto_resume_supported: None,
        profile_graceful_shutdown: None,
        codex_usage_account: None,
        agent_args: args.agent_args.clone(),
        initial_lineage,
        format: args.format,
        ordinary_start: Some(Box::new(args)),
    };
    let journal_id = lifecycle::start_id(
        context,
        import.id.as_deref(),
        import.agent,
        import.title.as_deref(),
    )?;
    lifecycle::attempt_created(context, &journal_id, "start", || {
        start_provider_resume_session(context, import)
    })
}

pub(super) fn conversation_lock(
    context: &CliContext,
    agent: AgentKind,
    id: &str,
) -> Result<fs::File, CliError> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{}:{id}", agent.as_str()).as_bytes());
    coordination::lock_private_store(
        context,
        &format!(
            "provider-resume-{}.lock",
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
    )
}

pub(super) fn bind_claude(
    record: &mut SessionRecord,
    initial: &InitialClaudeAccount,
    resumed: bool,
    config_dir: Option<&Path>,
) -> Result<(), CliError> {
    if resumed {
        claude_account::bind_prepared_initial(
            record,
            &initial.account,
            initial.selection_source,
            config_dir.ok_or_else(|| refusal("resume-account-mismatch"))?,
        )
    } else {
        claude_account::bind_initial(record, &initial.account, initial.selection_source)
    }
}
