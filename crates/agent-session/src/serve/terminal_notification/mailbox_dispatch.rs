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
    if let Err(error) = candidate.prepare_notice(&state.context, &record) {
        let detail = match error.code() {
            "notice-writers-disabled" => "notice-writers-disabled",
            "notice-native-unavailable" => "notice-runtime-unavailable",
            _ => "notice-not-prepared",
        };
        defer_provider_not_ready(&state, &candidate, detail).await;
        return;
    }
    if record.agent == AgentKind::Claude.as_str() {
        super::native_notice::dispatch(state, record, candidate).await;
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
        super::native_notice::dispatch(state, record, candidate).await;
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
        None,
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
            let refusal = refusal(&response);
            defer_refused(&state, &candidate, refusal).await;
        }
    } else {
        let refusal = refusal(&response);
        defer_refused(&state, &candidate, refusal).await;
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
            defer_provider_not_ready(&state, &candidate, "prompt-control-unavailable").await;
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
        let condition = quiescence.notification_block(true);
        if condition.is_some() || !quiescence.begin_notification_attempt(&fence)? {
            return Err(CliError::unavailable(
                "coordination-checkpoint-not-quiescent",
                "the recipient has an active claim or operation",
                condition.map(|condition| json!({ "condition": condition })),
            ));
        }
        drop(record_lock);
        Ok::<_, CliError>(())
    })
    .await;
    match locked {
        Ok(Ok(())) => {}
        Ok(Err(err)) if err.code() == "coordination-checkpoint-not-quiescent" => {
            let condition = err
                .details()
                .and_then(|details| details["condition"].as_str())
                .map(str::to_owned);
            defer_not_quiescent(&state, &candidate, condition).await;
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

/// Defers a wake as `provider-not-ready` and records which branch refused, so
/// `message audit` can tell the branches apart.
pub(super) async fn defer_provider_not_ready(
    state: &ServeState,
    candidate: &crate::coordination::NotificationCandidate,
    detail: &str,
) {
    let context = state.context.clone();
    let candidate = candidate.clone();
    let detail = detail.to_owned();
    let _ = tokio::task::spawn_blocking(move || {
        crate::coordination::defer_notification_with_detail(
            &context,
            &candidate,
            "provider-not-ready",
            Some(&detail),
            COORDINATION_NOTIFICATION_RETRY,
        )
    })
    .await;
}

/// How a refused structured prompt submission defers the wake.
enum Refusal {
    /// The recipient is not quiescent; carries the blocking condition.
    NotQuiescent(String),
    /// Control is missing or the submission was refused; carries the branch.
    ProviderNotReady(String),
}

fn refusal(response: &Response) -> Refusal {
    let error = response.extensions().get::<CliError>();
    if let Some(condition) = error
        .filter(|error| error.code() == crate::coordination::NOTIFICATION_NOT_QUIESCENT_CODE)
        .and_then(|error| error.details())
        .and_then(|details| details["condition"].as_str())
    {
        return Refusal::NotQuiescent(condition.to_owned());
    }
    Refusal::ProviderNotReady(match error.map(CliError::code) {
        Some("structured-prompt-unavailable") => "prompt-control-unavailable".to_owned(),
        Some(code) => format!("prompt-submission-refused:{code}"),
        None => "prompt-submission-refused".to_owned(),
    })
}

/// Defers a wake whose structured prompt submission was refused, under the
/// refusal's true reason: a recipient that is not quiescent reports the
/// blocking condition rather than `provider-not-ready`.
async fn defer_refused(
    state: &ServeState,
    candidate: &crate::coordination::NotificationCandidate,
    refusal: Refusal,
) {
    match refusal {
        Refusal::NotQuiescent(condition) => {
            defer_not_quiescent(state, candidate, Some(condition)).await;
        }
        Refusal::ProviderNotReady(detail) => {
            defer_provider_not_ready(state, candidate, &detail).await;
        }
    }
}

/// Defers a wake as `recipient-coordination-not-quiescent`, naming the
/// blocking condition when it is known.
async fn defer_not_quiescent(
    state: &ServeState,
    candidate: &crate::coordination::NotificationCandidate,
    condition: Option<String>,
) {
    let context = state.context.clone();
    let candidate = candidate.clone();
    let _ = tokio::task::spawn_blocking(move || {
        crate::coordination::defer_notification_with_detail(
            &context,
            &candidate,
            "recipient-coordination-not-quiescent",
            condition.as_deref(),
            COORDINATION_NOTIFICATION_RETRY,
        )
    })
    .await;
}
