//! Relaunch existing managed sessions, preserving owned lifecycle continuity.
use super::*;

pub(super) fn resume_session_by_id(
    context: &CliContext,
    id: &str,
    tmux_bin: &Path,
) -> Result<SessionView, CliError> {
    crate::lifecycle::attempt(context, id, "resume", || {
        resume_session_by_id_unjournaled(context, id, tmux_bin)
    })
}

fn resume_session_by_id_unjournaled(
    context: &CliContext,
    id: &str,
    tmux_bin: &Path,
) -> Result<SessionView, CliError> {
    // Serialize the entire launch and rollback against record writers.
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _record_lock = acquire_session_record_lock(context, &canonical_id)?;
    let record = load_session_record(context, &canonical_id)?;
    ensure_same_session_identity(&observed, &record)?;
    let outcome = resume_session_locked(context, record, tmux_bin)?;
    restart::release_bootstrap(outcome.bootstrap, _record_lock);
    Ok(outcome.session)
}

pub(super) struct ResumeSessionOutcome {
    pub(super) session: SessionView,
    pub(super) session_incarnation: Option<String>,
    pub(super) session_generation: Option<u64>,
    pub(super) bootstrap: Option<codex_app_server::CreateBootstrapGuard>,
}

pub(super) fn resume_session_locked(
    context: &CliContext,
    record: SessionRecord,
    tmux_bin: &Path,
) -> Result<ResumeSessionOutcome, CliError> {
    resume_session_with_kind(context, record, tmux_bin, provider_continuity::Kind::Resume)
}

pub(super) fn resume_session_with_kind(
    context: &CliContext,
    mut record: SessionRecord,
    tmux_bin: &Path,
    kind: provider_continuity::Kind,
) -> Result<ResumeSessionOutcome, CliError> {
    if session_model::is_retired_dsh_session(&record) {
        return Err(session_model::retired_dsh_session_error(&record));
    }
    ensure_session_lifecycle_mutation_allowed(context, &record)?;
    let _worktree_lifecycle = acquire_worktree_lifecycle(context, Path::new(&record.cwd))?;
    match restart::resume_status(tmux_bin, &record) {
        restart::ResumeStatus::Running => {
            return Ok(ResumeSessionOutcome {
                session: session_view(
                    context,
                    &record,
                    Some("running".to_string()),
                    Some(tmux_bin),
                ),
                session_incarnation: record
                    .runtime
                    .as_ref()
                    .map(|runtime| runtime.launch_id.clone())
                    .filter(|launch_id| !launch_id.is_empty()),
                session_generation: record.runtime.as_ref().map(|runtime| runtime.generation),
                bootstrap: None,
            });
        }
        status @ restart::ResumeStatus::Unknown { .. } => {
            return Err(CliError::runtime(
                "session-status-unknown",
                format!("session status could not be checked: {}", record.id),
                Some(status.retry_details(&record)),
            ));
        }
        _ => {}
    }
    let durable_profile_context = validate_durable_profile_resume_context(&record)?;
    StartupArtifactBackup::ensure_not_interrupted(context, &record)?;
    if record.provider_resume.is_none()
        && AgentKind::from_name(&record.agent) == Some(AgentKind::Codex)
        && !restart::fresh_startup(context, &record)
        && let Some(provider_resume) = capture_codex_resume_from_history(&record)
    {
        record.provider_resume = Some(provider_resume);
        write_session_record(context, &record)?;
    }
    let plan = restart::launch_plan(context, &record)?;
    let (agent, resume_args) = (plan.agent, plan.args.clone());
    if recorded_runtime_is_from_prior_boot(&record) {
        record.extra.remove(DELETE_TMUX_TERMINATION_STATE_KEY);
    }
    let prior_identities = persisted_prior_tmux_runtime_identities(&record).map_err(|reason| {
        session_termination_error(&record, reason, SessionTerminationOperation::Resume)
    })?;
    let termination_state = persisted_tmux_termination_state(&record).map_err(|reason| {
        session_termination_error(&record, reason, SessionTerminationOperation::Resume)
    })?;
    match persisted_tmux_runtime_identity(&record).map_err(|reason| {
        session_termination_error(&record, reason, SessionTerminationOperation::Resume)
    })? {
        Some(identity) => {
            if prior_identities
                .iter()
                .any(|prior| !prior.same_runtime_target(&identity))
            {
                return Err(session_termination_error(
                    &record,
                    SessionTerminationFailure::RuntimeIdentityMismatch,
                    SessionTerminationOperation::Resume,
                ));
            }
            let verification_started = Instant::now();
            verify_stopped_process_runtimes(&prior_identities, DELETE_TERMINATION_VERIFY_TIMEOUT)
                .map_err(|reason| {
                session_termination_error(&record, reason, SessionTerminationOperation::Resume)
            })?;
            // The sub-step above was given this budget, so it may have spent
            // it. The verification below now probes once regardless, which is
            // what decides the verdict; failing here instead reported a
            // termination that was never checked.
            let remaining =
                DELETE_TERMINATION_VERIFY_TIMEOUT.saturating_sub(verification_started.elapsed());
            verify_stopped_tmux_runtime(tmux_bin, &record.tmux_session, &identity, remaining)
                .map_err(|reason| {
                    session_termination_error(&record, reason, SessionTerminationOperation::Resume)
                })?;
        }
        None if runtime_is_proven_never_launched(&record) && prior_identities.is_empty() => {}
        None => {
            return Err(session_termination_error(
                &record,
                SessionTerminationFailure::RuntimeIdentityUnavailable,
                SessionTerminationOperation::Resume,
            ));
        }
    }
    if matches!(
        termination_state,
        Some(TmuxTerminationState::Pending { .. })
    ) {
        return Err(session_termination_error(
            &record,
            SessionTerminationFailure::RuntimeIdentityUnavailable,
            SessionTerminationOperation::Resume,
        ));
    }
    let (_conversation, _sources) =
        restart::conversation_guards(context, &record, &plan, tmux_bin)?;
    record.extra.remove(DELETE_TMUX_PRIOR_IDENTITIES_KEY);
    runtime_teardown::retire_confirmed_broker(context, &record, tmux_bin, termination_state)?;
    record.extra.remove(DELETE_TMUX_TERMINATION_STATE_KEY);
    let previous_record = record.clone();
    let previous_runtime =
        coordination::capture_stopped_runtime(context, &previous_record, tmux_bin)?;
    let app_server_managed = agent == AgentKind::Codex
        && (codex_app_server::runtime_is_supported(&previous_record)
            || codex_account::binding_is_present(&previous_record));
    let previous_activity = activity::capture_snapshot(context, &record.id)?;
    let mut startup_artifacts = StartupArtifactBackup::stage(context, &record)?;
    let agent_bin = restart::resume_agent_bin(durable_profile_context, &record, agent);
    let admission = admission::reserve(context, Some(tmux_bin), Some(&record.id))?;
    let now = Zoned::now();
    let next_generation = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.generation.saturating_add(1))
        .unwrap_or(1);
    record.runtime = Some(RuntimeInfo {
        kind: "tmux".to_string(),
        tmux_session: record.tmux_session.clone(),
        generation: next_generation,
        started_at: now.timestamp().to_string(),
        launch_id: uuid::Uuid::new_v4().to_string(),
        extra: record
            .runtime
            .as_ref()
            .map(|runtime| runtime.extra.clone())
            .unwrap_or_default(),
    });
    record.extra.remove(TMUX_RUNTIME_NEVER_LAUNCHED_KEY);
    store_startup_projection(
        &mut record,
        &starting_projection(&now.timestamp().to_string(), "record"),
    );
    if app_server_managed {
        codex_account::prepare_resume_runtime(context, &agent_bin, &mut record, &previous_record)?;
    }
    if agent == AgentKind::Claude {
        claude_account::prepare_launch(&mut record)?;
    }
    let bootstrap = plan
        .conversation
        .is_none()
        .then(|| codex_app_server::begin_create_bootstrap(&record))
        .transpose()?
        .flatten();
    record.updated_at = now.timestamp().to_string();
    provider_continuity::append(&previous_record, &mut record, kind);
    runtime_environment::write_launch_record(context, &mut record)?;
    drop(admission);
    activity::activate_runtime(context, &record)?;
    if let Err(error) = coordination::prepare(context, &record) {
        let _ = write_session_record(context, &previous_record);
        let _ = activity::restore_snapshot(context, &record.id, &previous_activity);
        let _ = startup_artifacts.restore();
        return Err(error);
    }
    let launch = if app_server_managed {
        start_interactive_tmux(
            tmux_bin,
            &agent_bin,
            agent,
            &context.state_dir,
            &record,
            &resume_args,
            &record.agent_args,
        )
    } else {
        start_resume_tmux(
            tmux_bin,
            &agent_bin,
            &context.state_dir,
            &record,
            &resume_args,
        )
    };
    let launch_error = match launch {
        Ok(identity) => {
            if let Err(err) = persist_launched_tmux_identity(context, &mut record, &identity) {
                match recover_failed_tmux_launch(
                    context,
                    &mut record,
                    tmux_bin,
                    Some(&identity),
                    SessionTerminationOperation::Resume,
                ) {
                    Ok(()) => Some(err),
                    Err(termination_err) => {
                        startup_artifacts.discard();
                        return Err(termination_err);
                    }
                }
            } else if let Err(err) = establish_coordination_broker_with_previous(
                context,
                &record,
                Some(&previous_runtime),
            ) {
                match recover_failed_tmux_launch(
                    context,
                    &mut record,
                    tmux_bin,
                    Some(&identity),
                    SessionTerminationOperation::Resume,
                ) {
                    Ok(()) => Some(err),
                    Err(termination_err) => {
                        startup_artifacts.discard();
                        return Err(termination_err);
                    }
                }
            } else if let Err(err) = release_held_runtime(
                context,
                &record,
                LaunchKind::for_restore(plan.conversation.is_some()),
            ) {
                match recover_failed_tmux_launch(
                    context,
                    &mut record,
                    tmux_bin,
                    Some(&identity),
                    SessionTerminationOperation::Resume,
                ) {
                    Ok(()) => Some(err),
                    Err(termination_err) => {
                        startup_artifacts.discard();
                        return Err(termination_err);
                    }
                }
            } else {
                None
            }
        }
        Err(err) if tmux_launch_may_have_created_runtime(&err) => {
            match recover_failed_tmux_launch(
                context,
                &mut record,
                tmux_bin,
                None,
                SessionTerminationOperation::Resume,
            ) {
                Ok(()) => Some(err),
                Err(termination_err) => {
                    startup_artifacts.discard();
                    return Err(termination_err);
                }
            }
        }
        Err(err) => Some(err),
    };
    if let Some(launch_err) = launch_error {
        let coordination_revoke = contention::revoke_failed_resume(context, &record, &launch_err);
        let record_restore = write_session_record(context, &previous_record);
        let activity_restore = activity::restore_snapshot(context, &record.id, &previous_activity);
        let artifact_restore = startup_artifacts.restore();
        if coordination_revoke.is_err()
            || record_restore.is_err()
            || activity_restore.is_err()
            || artifact_restore.is_err()
        {
            return Err(CliError::runtime(
                "resume-launch-rollback-failed",
                "provider resume launch failed and the prior durable runtime could not be fully restored",
                Some(json!({
                    "id": record.id,
                    "launch_error": launch_err.code(),
                    "new_coordination_revoked": coordination_revoke.is_ok_and(|revoked| revoked),
                    "record_restored": record_restore.is_ok(),
                    "activity_restored": activity_restore.is_ok(),
                    "startup_artifacts_restored": artifact_restore.is_ok()
                })),
            ));
        }
        return Err(contention::resume_error(launch_err));
    }
    if let (Ok(previous_incarnation), Ok(current_incarnation)) = (
        coordination::incarnation(&previous_record),
        coordination::incarnation(&record),
    ) && previous_incarnation != current_incarnation
    {
        let _ = fs::remove_file(coordination::checkpoint_path_for_state(
            &context.state_dir,
            &previous_record.id,
            &previous_incarnation,
        ));
    }
    reconcile_owned_startup_projection(context, &mut record, "running");
    startup_artifacts.discard();
    Ok(ResumeSessionOutcome {
        session: session_view(
            context,
            &record,
            Some("running".to_string()),
            Some(tmux_bin),
        ),
        session_incarnation: record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.clone())
            .filter(|launch_id| !launch_id.is_empty()),
        session_generation: record.runtime.as_ref().map(|runtime| runtime.generation),
        bootstrap,
    })
}

pub(super) fn render_glance_text(result: &GlanceResult) -> String {
    let mut text = format!("{} {} [{}]\n", result.id, result.agent, result.status);
    if let Some(notice) = &result.native_notice {
        text.push_str(&notice.text());
    }
    text.push_str(&result.tail);
    if !result.tail.is_empty() && !result.tail.ends_with('\n') {
        text.push('\n');
    }
    text
}

pub(super) fn render_resumed_text(result: &SessionView) -> String {
    let mut text = format!(
        "resumed {} session {}\ntmux: {}\nattach: {}\n",
        result.agent, result.id, result.tmux_session, result.attach_command
    );
    if let Some(notice) = &result.native_notice {
        text.push_str(&notice.text());
    }
    text
}
