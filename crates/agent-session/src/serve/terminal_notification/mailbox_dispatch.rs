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
    if crate::send::claude_notice::attach::mailbox_text_reason(&state.context, &record).is_some() {
        candidate.prompt = Some(candidate.text_wake_prompt());
        super::native_notice::dispatch(state, record, candidate).await;
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

    super::attested::dispatch(state, record, candidate, None).await;
}

async fn dispatch_codex_in_turn_coordination_checkpoint(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
    expected_turn_id: String,
) {
    super::attested::dispatch(state, record, candidate, Some(expected_turn_id)).await;
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
pub(super) enum Refusal {
    /// The recipient is not quiescent; carries the blocking condition.
    NotQuiescent(String),
    /// Control is missing or the submission was refused; carries the branch.
    ProviderNotReady(String),
}

pub(super) fn refusal(response: &Response) -> Refusal {
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
pub(super) async fn defer_refused(
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
