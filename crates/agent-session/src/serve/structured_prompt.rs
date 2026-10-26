use super::*;

#[allow(clippy::result_large_err)]
pub(super) async fn submit_structured_prompt_locked(
    state: &ServeState,
    id: &str,
    launch_id: &str,
    guards: StructuredPromptGuards<'_>,
    handle: &ControlHandle,
    text: &str,
) -> Result<StructuredPromptAcceptance, Response> {
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
    let history_session_id = record
        .provider_resume
        .as_ref()
        .filter(|resume| resume.provider == "codex")
        .map(|resume| resume.session_id.clone());
    let response = match handle.submit_prompt(text).await {
        Ok(history_turn_id) => Ok(StructuredPromptAcceptance {
            session_incarnation,
            history_turn_id: Some(history_turn_id),
            history_session_id,
        }),
        Err(error) => {
            let exited = matches!(error, crate::codex_app_server::prompt_submit::PromptSubmissionFailure::AcknowledgementUnknown)
                && recorded_runtime_stopped(state, &record).await;
            if exited {
                Ok(StructuredPromptAcceptance {
                    session_incarnation,
                    history_turn_id: None,
                    history_session_id: None,
                })
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

#[derive(Debug)]
pub(super) struct StructuredPromptAcceptance {
    pub(super) session_incarnation: String,
    pub(super) history_turn_id: Option<String>,
    pub(super) history_session_id: Option<String>,
}

/// Submit one structured prompt to a Claude session and report `submitted` only
/// once Claude's own `UserPromptSubmit` hook has produced a new turn, or the
/// pane or new transcript row shows it queued behind a running turn
/// (`queued: true`). A tmux
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
    let delivery = {
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
            Ok(Ok(delivery)) => delivery,
            Ok(Err(err)) => return envelope_err(err),
            Err(_) => return join_err(),
        }
    };

    match claude_prompt_ack::await_claude_prompt_ack(
        state,
        record,
        &delivery.turn_baseline,
        delivery.prompt_tail,
        delivery.queue_before.as_deref(),
        &delivery.acknowledgement_text,
        acknowledgement_wait,
    )
    .await
    {
        Ok(claude_prompt_ack::ClaudePromptAck::Turn) => {
            return envelope_ok(json!({
                "machine": state.machine,
                "submitted": true,
                "session_incarnation": delivery.session_incarnation,
            }));
        }
        // A prompt sent mid-turn starts no turn until the running one ends, but
        // Claude Code shows it queued; that is a delivery, not an unknown outcome.
        Ok(claude_prompt_ack::ClaudePromptAck::Queued) => {
            return envelope_ok(json!({
                "machine": state.machine,
                "submitted": true,
                "queued": true,
                "session_incarnation": delivery.session_incarnation,
            }));
        }
        Ok(claude_prompt_ack::ClaudePromptAck::Unknown) => {
            let exited = recorded_runtime_stopped(state, record).await;
            if exited {
                return envelope_ok(json!({
                    "machine": state.machine,
                    "submitted": true,
                    "session_incarnation": delivery.session_incarnation,
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

pub(super) async fn submit_structured_prompt_handler(
    state: Arc<ServeState>,
    id: String,
    text: String,
    expected_session_incarnation: Option<String>,
) -> Response {
    submit_structured_prompt_handler_with_fence(
        state,
        id,
        text,
        expected_session_incarnation,
        None,
        false,
    )
    .await
}

pub(super) async fn submit_structured_prompt_handler_for_receipt(
    state: Arc<ServeState>,
    id: String,
    text: String,
    expected_session_incarnation: Option<String>,
) -> Response {
    submit_structured_prompt_handler_with_fence(
        state,
        id,
        text,
        expected_session_incarnation,
        None,
        true,
    )
    .await
}

pub(super) async fn submit_structured_prompt_handler_with_fence(
    state: Arc<ServeState>,
    id: String,
    text: String,
    expected_session_incarnation: Option<String>,
    notification_fence: Option<CoordinationNotificationFence>,
    include_history_turn_id: bool,
) -> Response {
    if text.trim().is_empty() {
        return status_json(
            StatusCode::BAD_REQUEST,
            "empty-prompt",
            "structured prompt text must not be blank",
        );
    }
    if text.len() > MAX_PROVIDER_PROMPT_BYTES {
        return status_json(
            StatusCode::PAYLOAD_TOO_LARGE,
            "prompt-too-large",
            "structured prompt exceeds the maximum supported size",
        );
    }
    if structured_prompt_has_unsafe_control(&text) {
        return status_json(
            StatusCode::BAD_REQUEST,
            "unsafe-prompt-control",
            "structured prompt contains an unsupported control character",
        );
    }
    let expected_session_incarnation = match expected_session_incarnation.as_deref() {
        Some(value) if value.trim().is_empty() || value.len() > 128 => {
            return status_json(
                StatusCode::BAD_REQUEST,
                "invalid-session-incarnation",
                "expected_session_incarnation must be a nonblank identifier of at most 128 bytes",
            );
        }
        value => value,
    };

    let record =
        match load_structured_prompt_record_locked(&state, &id, expected_session_incarnation).await
        {
            Ok(record) => record,
            Err(response) => return response,
        };
    let Some(launch_id) = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
    else {
        return status_json(
            StatusCode::CONFLICT,
            "structured-prompt-unsupported",
            "this session does not provide structured prompt submission",
        );
    };
    // Coordination notifications are fenced against the Codex app-server's own
    // turn state, so a fenced request stays Codex-only until that fence has a
    // Claude equivalent.
    if notification_fence.is_none() && crate::claude_structured_prompt_supported(&record) {
        // Only this branch pastes the prompt into the pane, so only this branch
        // carries the blocked-input hazard. The Codex branch below submits over
        // the app-server control channel and never touches the terminal, which
        // is why it is not refused here — the daemon's own in-turn coordination
        // checkpoint deliberately delivers to a `NeedsInput` Codex session over
        // that same channel. This is an early reject only; the authoritative
        // check lives inside `deliver_claude_structured_prompt_locked`, under
        // the record lock that also fences the write.
        if let Err(err) = crate::activity::refuse_if_blocked(
            &state.context,
            &record,
            "answer the dialog with send --key, then resubmit the prompt",
        ) {
            return envelope_err(err);
        }
        return submit_claude_structured_prompt(
            &state,
            &record,
            &launch_id,
            expected_session_incarnation,
            &text,
            CLAUDE_STRUCTURED_PROMPT_ACK_WAIT,
        )
        .await;
    }
    if !codex_app_server::runtime_is_supported(&record) {
        return status_json(
            StatusCode::CONFLICT,
            "structured-prompt-unsupported",
            "this session does not provide structured prompt submission",
        );
    }

    let handle = match wait_for_structured_prompt_control(
        &state,
        &id,
        &launch_id,
        expected_session_incarnation,
        STRUCTURED_PROMPT_CONTROL_WAIT,
    )
    .await
    {
        Ok(handle) => handle,
        Err(response) => return response,
    };

    match submit_structured_prompt_locked(
        &state,
        &record.id,
        &launch_id,
        StructuredPromptGuards {
            expected_session_incarnation,
            notification: notification_fence,
            ..Default::default()
        },
        &handle,
        &text,
    )
    .await
    {
        Ok(accepted) => {
            let mut data = json!({
                "machine": state.machine,
                "submitted": true,
                "session_incarnation": accepted.session_incarnation,
            });
            if include_history_turn_id && let Some(history_turn_id) = accepted.history_turn_id {
                data["history_turn_id"] = json!(history_turn_id);
                if let Some(history_session_id) = accepted.history_session_id {
                    data["history_session_id"] = json!(history_session_id);
                }
            }
            envelope_ok(data)
        }
        Err(response) => response,
    }
}
