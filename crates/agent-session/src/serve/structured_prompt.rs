use super::*;

#[allow(clippy::result_large_err)]
pub(super) async fn submit_structured_prompt_locked(
    state: &ServeState,
    id: &str,
    launch_id: &str,
    guards: StructuredPromptGuards<'_>,
    handle: &ControlHandle,
    text: &str,
) -> Result<String, Response> {
    let readiness = handle.wait_ready(CODEX_CREATE_PROMPT_CONTROL_WAIT).await;
    if readiness.is_err() {
        load_structured_prompt_record_locked(state, id, Some(launch_id)).await?;
        return Err(status_json(
            StatusCode::CONFLICT,
            "structured-prompt-unavailable",
            "structured prompt control is not ready for this session",
        ));
    }
    let lock_context = state.context.clone();
    let tmux_bin = state.tmux_bin.clone();
    let lock_id = id.to_string();
    let expected_launch_id = launch_id.to_string();
    let expected_session_incarnation = guards.expected_session_incarnation.map(str::to_string);
    let expected_binding = guards
        .expected_binding
        .map(|(account, revision)| (account.to_string(), revision));
    let direct_reply = guards.expected_binding.is_none() && guards.notification.is_none();
    let notification_fence = guards.notification;
    let (record_lock, session_incarnation, record) = match tokio::task::spawn_blocking(move || {
        let record_lock = crate::acquire_session_record_lock(&lock_context, &lock_id)?;
        let mut current = load_session_record(&lock_context, &lock_id)?;
        let Some(actual_launch_id) = current
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.clone())
        else {
            return Err(structured_prompt_incarnation_conflict(
                &current.id,
                &expected_launch_id,
                None,
            ));
        };
        if actual_launch_id != expected_launch_id {
            return Err(structured_prompt_incarnation_conflict(
                &current.id,
                &expected_launch_id,
                Some(&actual_launch_id),
            ));
        }
        if let Some(expected) = expected_session_incarnation
            && actual_launch_id != expected
        {
            return Err(structured_prompt_incarnation_conflict(
                &current.id,
                &expected,
                Some(&actual_launch_id),
            ));
        }
        if let Some((account, revision)) = expected_binding
            && crate::codex_account::binding_snapshot(&current)
                != (crate::codex_account::BindingSnapshot::Bound { account, revision })
        {
            return Err(CliError::data(
                "codex-account-binding-superseded",
                "the requested Codex account binding was superseded before prompt submission",
                Some(json!({ "id": current.id })),
            ));
        }
        codex_quiet::precheck(&lock_context, &current, notification_fence.is_some())?;
        if direct_reply {
            crate::send_guard::require_empty_composer(&current, &tmux_bin)?;
        }
        crate::codex_account::authorize_input_locked(&lock_context, &mut current)?;
        auto_resume::cancel_for_manual_input_locked(
            &lock_context,
            &current.id,
            &jiff::Timestamp::now().to_string(),
        )?;
        crate::codex_account::ensure_input_allowed(&current)?;
        if let Some(fence) = notification_fence {
            let eligible = codex_quiet::fenced_eligible(&lock_context, &current, &fence);
            let mut quiescence = crate::coordination::lock_session_quiescence(
                &lock_context,
                &fence.candidate.target_session_id,
                &fence.candidate.target_incarnation,
            )?;
            if !eligible || !quiescence.begin_notification_attempt(&fence.candidate)? {
                return Err(CliError::data(
                    "coordination-notification-superseded",
                    "notification eligibility changed before locked prompt submission",
                    None,
                ));
            }
        }
        Ok::<_, CliError>((record_lock, actual_launch_id, current))
    })
    .await
    {
        Ok(Ok(locked)) => locked,
        Ok(Err(err)) if err.code() == "codex-account-binding-superseded" => {
            return Err(status_json(
                StatusCode::CONFLICT,
                err.code(),
                "session account changed before prompt submission",
            ));
        }
        Ok(Err(err)) => return Err(envelope_err(err)),
        Err(_) => return Err(join_err()),
    };
    let response = match handle.submit_prompt(text).await {
        Ok(_) => Ok(session_incarnation),
        Err(error) => {
            let exited = matches!(error, crate::codex_app_server::prompt_submit::PromptSubmissionFailure::AcknowledgementUnknown)
                && recorded_runtime_stopped(state, &record).await;
            if exited {
                Ok(session_incarnation)
            } else {
                Err(status_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "structured-prompt-outcome-unknown",
                    "structured prompt submission outcome is unknown",
                ))
            }
        }
    };
    drop(record_lock);
    response
}

/// Submit one structured prompt to a Claude session and report `submitted` only
/// once Claude's own `UserPromptSubmit` hook has produced a new turn, or the
/// pane shows the prompt queued behind a running turn (`queued: true`). A tmux
/// write that is neither is reported as an unknown outcome, never as a
/// submission.
pub(super) async fn submit_claude_structured_prompt(
    state: &Arc<ServeState>,
    record: &SessionRecord,
    launch_id: &str,
    expected_session_incarnation: Option<&str>,
    text: &str,
    acknowledgement_wait: Duration,
) -> Response {
    let baseline = {
        let context = state.context.clone();
        let record = record.clone();
        match tokio::task::spawn_blocking(move || {
            crate::activity::prompt_turn_baseline(&context, &record)
        })
        .await
        {
            Ok(baseline) => baseline,
            Err(_) => return join_err(),
        }
    };

    let session_incarnation = {
        let context = state.context.clone();
        let id = record.id.clone();
        let expected = expected_session_incarnation
            .unwrap_or(launch_id)
            .to_string();
        let text = text.to_string();
        let tmux_bin = state.tmux_bin.clone();
        match tokio::task::spawn_blocking(move || {
            crate::deliver_claude_structured_prompt_locked(
                &context, &id, &expected, &text, &tmux_bin,
            )
        })
        .await
        {
            Ok(Ok(session_incarnation)) => session_incarnation,
            Ok(Err(err)) => return envelope_err(err),
            Err(_) => return join_err(),
        }
    };

    match claude_prompt_ack::await_claude_prompt_ack(
        state,
        record,
        &baseline,
        text,
        acknowledgement_wait,
    )
    .await
    {
        Ok(claude_prompt_ack::ClaudePromptAck::Turn) => {
            return envelope_ok(json!({
                "machine": state.machine,
                "submitted": true,
                "session_incarnation": session_incarnation,
            }));
        }
        // A prompt sent mid-turn starts no turn until the running one ends, but
        // Claude Code shows it queued; that is a delivery, not an unknown outcome.
        Ok(claude_prompt_ack::ClaudePromptAck::Queued) => {
            return envelope_ok(json!({
                "machine": state.machine,
                "submitted": true,
                "queued": true,
                "session_incarnation": session_incarnation,
            }));
        }
        Ok(claude_prompt_ack::ClaudePromptAck::Unknown) => {
            let exited = recorded_runtime_stopped(state, record).await;
            if exited {
                return envelope_ok(json!({
                    "machine": state.machine,
                    "submitted": true,
                    "session_incarnation": session_incarnation,
                }));
            }
        }
        Err(()) => return join_err(),
    }
    status_json(
        StatusCode::INTERNAL_SERVER_ERROR,
        "structured-prompt-outcome-unknown",
        "structured prompt submission outcome is unknown",
    )
}

async fn recorded_runtime_stopped(state: &ServeState, record: &SessionRecord) -> bool {
    let tmux = state.tmux_bin.clone();
    let record = record.clone();
    tokio::task::spawn_blocking(move || {
        crate::send::recorded_tmux_runtime_stopped(
            &tmux,
            &record,
            crate::PANE_INPUT_COMMAND_TIMEOUT,
        )
    })
    .await
    .unwrap_or(false)
}
