//! Controller notice admission and confirmation share the bounded Codex owner.
use super::*;
use crate::send::codex_notice::Prepared;
use std::time::Instant;

pub(super) async fn dispatch(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
    expected_turn: Option<String>,
) {
    let response = submit(&state, &record, &candidate, expected_turn).await;
    let context = state.context.clone();
    if response.status().is_success() {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        })
        .await;
    } else {
        let attempted = candidate.clone();
        let claimed = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_unknown(
                &context,
                &attempted,
                "submission-outcome-unknown",
            )
        })
        .await;
        if matches!(claimed, Ok(Ok(false))) {
            let refusal = super::mailbox_dispatch::refusal(&response);
            super::mailbox_dispatch::defer_refused(&state, &candidate, refusal).await;
        }
    }
}

async fn submit(
    state: &Arc<ServeState>,
    record: &SessionRecord,
    candidate: &crate::coordination::NotificationCandidate,
    expected_turn: Option<String>,
) -> Response {
    let launch = candidate.target_incarnation.clone();
    let handle = match wait_for_structured_prompt_control(
        state,
        &record.id,
        &launch,
        Some(&launch),
        STRUCTURED_PROMPT_CONTROL_WAIT,
    )
    .await
    {
        Ok(handle) => handle,
        Err(response) => return response,
    };
    let started = Instant::now();
    let deadline = started + Duration::from_secs(15);
    let context = state.context.clone();
    let expected = record.clone();
    let fence = candidate.clone();
    let turn = expected_turn.clone();
    let text = crate::coordination::notification_prompt(candidate);
    let body = text.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let record_lock = crate::acquire_session_record_lock_timed(
            &context,
            &expected.id,
            deadline.saturating_duration_since(Instant::now()),
        )?;
        let mut current = load_session_record(&context, &expected.id)?;
        let identity = prompt_receipts::native_identity(&expected);
        if identity.is_none()
            || prompt_receipts::native_identity(&current) != identity
            || !codex_app_server::runtime_is_supported(&current)
        {
            return Err(crate::send::codex_notice::unconfirmed());
        }
        if let Some(expected) = turn.as_deref() {
            let eligible = activity::state_for_view(&context, &current).is_some_and(|active| {
                matches!(
                    active.phase,
                    activity::TurnPhase::Working | activity::TurnPhase::NeedsInput
                ) && active.source.confidence == activity::Confidence::Authoritative
                    && active
                        .current_turn
                        .as_ref()
                        .and_then(|turn| turn.provider_turn_id.as_deref())
                        == Some(expected)
            });
            if !eligible {
                return Err(CliError::unavailable(
                    "coordination-checkpoint-turn-changed",
                    "the active turn changed before the coordination checkpoint",
                    None,
                ));
            }
            crate::codex_account::authorize_terminal_input_locked(&context, &mut current)?;
            crate::codex_account::ensure_terminal_input_allowed(&current)?;
        } else {
            codex_quiet::precheck(&context, &current, true)?;
            if !codex_quiet::fenced_eligible(
                &context,
                &current,
                &CoordinationNotificationFence {
                    candidate: fence.clone(),
                },
            ) {
                return Err(CliError::unavailable(
                    "coordination-notification-superseded",
                    "notification idle eligibility changed",
                    None,
                ));
            }
            crate::codex_account::authorize_input_locked(&context, &mut current)?;
            auto_resume::cancel_for_manual_input_locked(
                &context,
                &current.id,
                &jiff::Timestamp::now().to_string(),
            )?;
            crate::codex_account::ensure_input_allowed(&current)?;
        }
        let mut quiescence = crate::coordination::lock_session_quiescence(
            &context,
            &fence.target_session_id,
            &fence.target_incarnation,
        )?;
        if let Some(condition) = quiescence.notification_block(turn.is_some()) {
            return Err(CliError::unavailable(
                crate::coordination::NOTIFICATION_NOT_QUIESCENT_CODE,
                "the recipient is not quiescent for a notification",
                Some(json!({"condition":condition})),
            ));
        }
        let prepared = Prepared::new(&context, &current, &body, started, deadline, None, || {
            let admitted = if turn.is_some() {
                quiescence.begin_notification_attempt(&fence)?
            } else {
                quiescence.begin_idle_notification_attempt(&fence)?
            };
            if admitted {
                Ok(())
            } else {
                Err(CliError::unavailable(
                    "coordination-notification-superseded",
                    "notification eligibility changed",
                    None,
                ))
            }
        })?;
        drop(quiescence);
        Ok::<_, CliError>((record_lock, current, prepared))
    })
    .await;
    let (record_lock, current, prepared) = match prepared {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => return envelope_err(error),
        Err(_) => return join_err(),
    };
    let write = codex_app_server::mailbox_notice::Write {
        expected: Box::new(current),
        candidate: candidate.clone(),
        expected_turn: expected_turn.clone(),
        deadline,
    };
    // The receiver reacquires and revalidates the lock at the actual write.
    drop(record_lock);
    let submitted = if let Some(turn) = expected_turn.as_deref() {
        handle
            .steer_prompt(&text, turn, Some(prepared.client_id()), Some(write))
            .await
    } else {
        handle
            .mailbox_prompt(&text, prepared.client_id(), write)
            .await
            .map_err(|_| "mailbox notice unconfirmed".to_string())
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || match submitted {
        Ok(turn) => prepared.confirm(&context, &text, &turn),
        Err(_) => Err(prepared.unknown(&context)),
    })
    .await
    {
        Ok(Ok(())) => envelope_ok(json!({"submitted":true})),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}
