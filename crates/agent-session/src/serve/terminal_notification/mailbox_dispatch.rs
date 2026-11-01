//! Mailbox dispatch preserves each provider semantic path and generation fence.
use super::*;

pub(in crate::serve) async fn dispatch_coordination_notification(
    state: Arc<ServeState>,
    mut candidate: crate::coordination::NotificationCandidate,
) {
    let context = state.context.clone();
    let target_session_id = candidate.target_session_id.clone();
    let record = tokio::task::spawn_blocking(move || {
        let _record_lock = crate::acquire_session_record_lock(&context, &target_session_id)?;
        load_session_record(&context, &target_session_id)
    })
    .await;
    let record = match record {
        Ok(Ok(record)) => record,
        Ok(Err(_)) | Err(_) => {
            update_notification_undeliverable(&state, &candidate, "recipient-incarnation-replaced")
                .await;
            return;
        }
    };
    if record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        != Some(candidate.target_incarnation.as_str())
    {
        update_notification_undeliverable(&state, &candidate, "recipient-incarnation-replaced")
            .await;
        return;
    }
    if record.coordination_mode == cli::CoordinationMode::Off {
        update_notification_undeliverable(&state, &candidate, "coordination-disabled").await;
        return;
    }
    // DSH (an external `dsh` lane or a dsh launch-profile pane) has no serve
    // prompt route; its policy hook claims the reminder at a model-step
    // boundary through `message reminder`.
    if record.agent == AgentKind::Dsh.as_str() {
        update_notification_undeliverable(
            &state,
            &candidate,
            crate::coordination::NOTIFICATION_REASON_HOOK_DELIVERED,
        )
        .await;
        return;
    }
    if coordination_notification_provider_unsupported(&record) {
        update_notification_undeliverable(&state, &candidate, "provider-unsupported").await;
        return;
    }
    if candidate.prepare_notice(&state.context, &record).is_err() {
        update_notification_deferred(
            &state,
            &candidate,
            "provider-not-ready",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
        return;
    }
    if record.agent == AgentKind::Claude.as_str() {
        dispatch_terminal_coordination_notification(state, record, candidate, AgentKind::Claude)
            .await;
        return;
    }
    let turn = activity::state_for_view(&state.context, &record);
    let waiting = turn
        .as_ref()
        .is_some_and(activity::codex_notification_maybe_idle);
    if codex_app_server::runtime_is_supported(&record)
        && let Some(active_turn_id) = turn.as_ref().and_then(|turn| {
            (matches!(
                turn.phase,
                activity::TurnPhase::Working | activity::TurnPhase::NeedsInput
            ) && turn.source.confidence == activity::Confidence::Authoritative)
                .then(|| {
                    turn.current_turn
                        .as_ref()
                        .and_then(|current| current.provider_turn_id.clone())
                })
                .flatten()
        })
    {
        dispatch_codex_in_turn_coordination_checkpoint(state, record, candidate, active_turn_id)
            .await;
        return;
    }
    if !waiting {
        update_notification_deferred(
            &state,
            &candidate,
            "recipient-working",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
        return;
    }
    if !codex_app_server::runtime_is_supported(&record) {
        dispatch_terminal_coordination_notification(state, record, candidate, AgentKind::Codex)
            .await;
        return;
    }

    let prompt = crate::coordination::notification_prompt(&candidate);
    let response = structured_prompt::submit_structured_prompt_handler_with_fence(
        state.clone(),
        candidate.target_session_id.clone(),
        prompt,
        Some(candidate.target_incarnation.clone()),
        Some(CoordinationNotificationFence {
            candidate: candidate.clone(),
        }),
        false,
    )
    .await;
    let context = state.context.clone();
    if response.status().is_success() {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        })
        .await;
    } else if candidate.is_notice() || response.status() == StatusCode::INTERNAL_SERVER_ERROR {
        let attempted_candidate = candidate.clone();
        let attempted = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_unknown(
                &context,
                &attempted_candidate,
                "submission-outcome-unknown",
            )
        })
        .await;
        if candidate.is_notice() && matches!(attempted, Ok(Ok(false))) {
            update_notification_deferred(
                &state,
                &candidate,
                "provider-not-ready",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
        }
    } else {
        update_notification_deferred(
            &state,
            &candidate,
            "provider-not-ready",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
    }
}

async fn dispatch_codex_in_turn_coordination_checkpoint(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
    expected_turn_id: String,
) {
    let launch_id = candidate.target_incarnation.clone();
    let handle = match wait_for_structured_prompt_control(
        &state,
        &record.id,
        &launch_id,
        Some(&launch_id),
        STRUCTURED_PROMPT_CONTROL_WAIT,
    )
    .await
    {
        Ok(handle) => handle,
        Err(_) => {
            update_notification_deferred(
                &state,
                &candidate,
                "provider-not-ready",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
    };
    let prompt = crate::coordination::notification_prompt(&candidate);
    let lock_context = state.context.clone();
    let lock_id = record.id.clone();
    let lock_launch_id = launch_id.clone();
    let lock_turn_id = expected_turn_id.clone();
    let fence = candidate.clone();
    let locked = tokio::task::spawn_blocking(move || {
        let record_lock = crate::acquire_session_record_lock(&lock_context, &lock_id)?;
        let mut current = load_session_record(&lock_context, &lock_id)?;
        if current
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str())
            != Some(lock_launch_id.as_str())
            || !codex_app_server::runtime_is_supported(&current)
        {
            return Err(structured_prompt_incarnation_conflict(
                &current.id,
                &lock_launch_id,
                current
                    .runtime
                    .as_ref()
                    .map(|runtime| runtime.launch_id.as_str()),
            ));
        }
        let eligible = activity::state_for_view(&lock_context, &current).is_some_and(|turn| {
            matches!(
                turn.phase,
                activity::TurnPhase::Working | activity::TurnPhase::NeedsInput
            ) && turn.source.confidence == activity::Confidence::Authoritative
                && turn
                    .current_turn
                    .as_ref()
                    .and_then(|active| active.provider_turn_id.as_deref())
                    == Some(lock_turn_id.as_str())
        });
        if !eligible {
            return Err(CliError::unavailable(
                "coordination-checkpoint-turn-changed",
                "the active turn changed before the coordination checkpoint",
                None,
            ));
        }
        crate::codex_account::authorize_terminal_input_locked(&lock_context, &mut current)?;
        crate::codex_account::ensure_terminal_input_allowed(&current)?;
        let mut quiescence = crate::coordination::lock_session_quiescence(
            &lock_context,
            &fence.target_session_id,
            &fence.target_incarnation,
        )?;
        if !quiescence.begin_notification_attempt(&fence)? {
            return Err(CliError::unavailable(
                "coordination-checkpoint-not-quiescent",
                "the recipient has an active claim or operation",
                None,
            ));
        }
        drop(record_lock);
        Ok::<_, CliError>(())
    })
    .await;
    match locked {
        Ok(Ok(())) => {}
        Ok(Err(err)) if err.code() == "coordination-checkpoint-not-quiescent" => {
            update_notification_deferred(
                &state,
                &candidate,
                "recipient-coordination-not-quiescent",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
        Ok(Err(_)) | Err(_) => {
            update_notification_deferred(
                &state,
                &candidate,
                "recipient-working",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
    };

    let response = handle.steer_prompt(&prompt, &expected_turn_id).await;
    let context = state.context.clone();
    if response.as_deref() == Ok(expected_turn_id.as_str()) {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        })
        .await;
    } else {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_unknown(
                &context,
                &candidate,
                "submission-outcome-unknown",
            )
        })
        .await;
    }
}

pub(in crate::serve) async fn dispatch_terminal_coordination_notification(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
    expected_agent: AgentKind,
) {
    if candidate.is_notice() {
        super::native_notice::dispatch(state, record, candidate).await;
        return;
    }
    let Some(source) = ProviderPromptTail::resolve_source(&record) else {
        update_notification_deferred(
            &state,
            &candidate,
            "provider-observation-unavailable",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
        return;
    };
    let Some(tail) = ProviderPromptTail::open_source_at_eof(source) else {
        update_notification_deferred(
            &state,
            &candidate,
            "provider-observation-unavailable",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
        return;
    };
    let prompt = crate::coordination::notification_prompt(&candidate);
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let start_candidate = candidate.clone();
    let start_prompt = prompt.clone();
    let started = tokio::task::spawn_blocking(move || {
        start_terminal_coordination_notification(
            &context,
            &tmux,
            &start_candidate,
            &start_prompt,
            tail,
            expected_agent,
        )
    })
    .await;
    let tail = match started {
        Ok(Ok(tail)) => tail,
        Ok(Err(TerminalNotificationStartError::Known(reason))) => {
            update_notification_deferred(
                &state,
                &candidate,
                reason,
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
        Ok(Err(TerminalNotificationStartError::UnknownReason(reason))) => {
            terminal_notification::mark_unknown_after_paste(&state, candidate, reason).await;
            return;
        }
        Ok(Err(TerminalNotificationStartError::Unknown)) | Err(_) => {
            terminal_notification::mark_unknown(&state, candidate, "submission-outcome-unknown")
                .await;
            return;
        }
    };

    let observed_prompt = prompt.clone();
    let observed = tokio::task::spawn_blocking(move || {
        observe_terminal_coordination_prompt(tail, &observed_prompt)
    })
    .await
    .unwrap_or(false);
    let context = state.context.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if observed {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        } else {
            crate::coordination::mark_notification_unknown(
                &context,
                &candidate,
                "submission-outcome-unknown",
            )
        }
    })
    .await;
}
