//! Terminal/native notices reuse guarded send without a second prompt route.
use super::*;

const NOT_READY: &str = "mailbox-notice-not-ready";

/// Called by guarded send while it holds the recipient record lock, after
/// identity/idle/composer admission and before releasing any notice text.
pub(crate) fn begin_mailbox_notice(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    candidate: &crate::coordination::NotificationCandidate,
    native_idle: bool,
) -> Result<(), CliError> {
    let refused = |reason: Option<&'static str>| {
        CliError::unavailable(
            NOT_READY,
            "mailbox notice eligibility changed",
            reason.map(|reason| json!({"reason":reason})),
        )
    };
    let agent = AgentKind::from_name(&record.agent).ok_or_else(|| refused(None))?;
    let text_wake = candidate.is_text_wake()
        && (native_idle
            || crate::send::claude_notice::attach::mailbox_text_reason(context, record).is_some());
    if (!candidate.is_notice() && !text_wake)
        || record.id != candidate.target_session_id
        || record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str())
            != Some(candidate.target_incarnation.as_str())
        || record.coordination_mode == cli::CoordinationMode::Off
    {
        return Err(refused(None));
    }
    // A replacement text wake has already passed exact incarnation, native idle
    // and empty-composer checks under the record lock in guarded send.
    if !(native_idle && text_wake) && !terminal_notification_waiting(context, record, agent) {
        return Err(refused(Some("notice-idle-unsettled")));
    }
    match tmux_session_attached(tmux, &record.tmux_session) {
        Ok(false) => {}
        Ok(true) => return Err(refused(Some("notice-recipient-attached"))),
        Err(()) => return Err(refused(Some("notice-attachment-unobservable"))),
    }
    if agent == AgentKind::Claude && !text_wake {
        crate::send::claude_notice::runtime::require_readiness(context, record)?;
    }
    let mut guard = crate::coordination::lock_session_quiescence(
        context,
        &candidate.target_session_id,
        &candidate.target_incarnation,
    )
    .map_err(|error| {
        if agent == AgentKind::Claude && !text_wake {
            refused(Some("notice-coordination-unobservable"))
        } else {
            error
        }
    })?;
    if let Some(condition) = guard.notification_block(false) {
        return Err(CliError::unavailable(
            crate::coordination::NOTIFICATION_NOT_QUIESCENT_CODE,
            "the recipient is not quiescent for a notification",
            Some(json!({"condition":condition})),
        ));
    }
    if !guard.begin_idle_notification_attempt(candidate)? {
        return Err(refused(None));
    }
    Ok(())
}

pub(super) async fn dispatch(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
    native_idle: bool,
) {
    let claude_native = record.agent == AgentKind::Claude.as_str() && candidate.is_notice();
    let text_reason = (candidate.is_text_wake() && !native_idle)
        .then(|| crate::send::claude_notice::attach::mailbox_text_reason(&state.context, &record))
        .flatten();
    // A terminal-started Codex record has no provider binding and no
    // structured prompt route, so an unobservable composer may never recover.
    let unbound_codex =
        record.agent == AgentKind::Codex.as_str() && record.provider_resume.is_none();
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let sending = candidate.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut args = crate::send::unguarded_args(
            record.id,
            Some(crate::coordination::notification_prompt(&sending)),
            vec![cli::SpecialKey::Enter],
            false,
            tmux,
        );
        args.guards = crate::send_guard::GuardArgs {
            claude_mailbox_native_idle: native_idle,
            notice_envelope: sending.is_notice(),
            require_idle: true,
            require_empty_composer: true,
            expect_runtime: Some(sending.target_incarnation.clone()),
            expect_provider_session: record.provider_resume.map(|provider| provider.session_id),
            // Claude uses the generation CAS at the release boundary; consuming
            // a separate key before that CAS would prevent a refused retry.
            idempotency_key: (record.agent != "claude").then(|| {
                format!(
                    "mailbox-{}-{}",
                    sending.target_incarnation, sending.generation
                )
            }),
            ..Default::default()
        };
        crate::send::send_mailbox_notice(&context, args, &sending)
    })
    .await;
    let context = state.context.clone();
    if matches!(result, Ok(Ok(_))) {
        let candidate = candidate.clone();
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        })
        .await;
    } else {
        // This transition only matches an attempted generation. A preclaim
        // refusal stays queued; a claimed failure/unknown stays consumed.
        let attempted_candidate = candidate.clone();
        let attempted = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_unknown(
                &context,
                &attempted_candidate,
                "submission-outcome-unknown",
            )
        })
        .await;
        if matches!(attempted, Ok(Ok(false))) {
            let composer_refusal = match &result {
                Ok(Err(error)) if native_idle && error.code() == "send-composer-not-empty" => {
                    Some(("recipient-composer-not-empty", None))
                }
                Ok(Err(error))
                    if error.code() == "notice-native-unavailable"
                        && error
                            .details()
                            .and_then(|details| details["reason"].as_str())
                            == Some("authorization-composer-not-empty") =>
                {
                    Some(("recipient-composer-not-empty", None))
                }
                Ok(Err(error))
                    if error.code() == "send-composer-unobservable"
                        && error
                            .details()
                            .and_then(|details| details["observation"].as_str())
                            == Some("region-not-single-row") =>
                {
                    Some((
                        "recipient-composer-unobservable",
                        Some("region-not-single-row"),
                    ))
                }
                Ok(Err(error)) if native_idle && error.code() == "send-composer-unobservable" => {
                    Some(("recipient-composer-unobservable", None))
                }
                _ => None,
            };
            let composer_unobservable = matches!(&result, Ok(Err(error))
                if error.code() == "send-composer-unobservable");
            if unbound_codex && composer_unobservable {
                let context = state.context.clone();
                let candidate = candidate.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    crate::coordination::defer_notification_unbound_unobservable_composer(
                        &context,
                        &candidate,
                        COORDINATION_NOTIFICATION_RETRY,
                    )
                })
                .await;
            } else if let Some((reason, detail)) = composer_refusal {
                let context = state.context.clone();
                let candidate = candidate.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    crate::coordination::defer_notification_with_detail(
                        &context,
                        &candidate,
                        reason,
                        detail,
                        COORDINATION_NOTIFICATION_RETRY,
                    )
                })
                .await;
            } else {
                let refusal = match &result {
                    Ok(Err(error)) => preclaim_refusal(error, claude_native || native_idle),
                    _ => provider_not_ready("notice-attempt-not-admitted"),
                };
                super::mailbox_dispatch::defer_refused(&state, &candidate, refusal).await;
            }
        }
    }
    if let Some(reason) = text_reason {
        let context = state.context.clone();
        let _ = tokio::task::spawn_blocking(move || {
            candidate.record_text_wake_reason(&context, reason)
        })
        .await;
    }
}

fn provider_not_ready(detail: &str) -> super::mailbox_dispatch::Refusal {
    super::mailbox_dispatch::Refusal::ProviderNotReady(detail.to_owned())
}

pub(super) fn module_readiness_detail(error: &CliError) -> Option<&'static str> {
    if error.code() != "notice-native-unavailable" {
        return None;
    }
    match error.details()?.get("reason")?.as_str()? {
        "module-readiness-unavailable" => Some("notice-runtime-unavailable"),
        "module-binding-mismatch" | "readiness-changed" | "authorization-binding" => {
            Some("notice-module-binding-mismatch")
        }
        _ => None,
    }
}

fn preclaim_refusal(error: &CliError, claude_native: bool) -> super::mailbox_dispatch::Refusal {
    use super::mailbox_dispatch::{Refusal, not_quiescent_condition};

    if let Some(reason) = codex_app_server::guarded::IdleReason::from_error(error) {
        return Refusal::ProviderNotReady(reason.notification_detail());
    }
    if !claude_native {
        return provider_not_ready("notice-attempt-not-admitted");
    }
    if let Some(condition) = not_quiescent_condition(error) {
        return Refusal::NotQuiescent(condition.to_owned());
    }
    if let Some(detail) = module_readiness_detail(error) {
        return provider_not_ready(detail);
    }
    let reason = error
        .details()
        .and_then(|details| details["reason"].as_str());
    provider_not_ready(match error.code() {
        NOT_READY => match reason {
            Some("notice-idle-unsettled") => "notice-idle-unsettled",
            Some("notice-recipient-attached") => "notice-recipient-attached",
            Some("notice-attachment-unobservable") => "notice-attachment-unobservable",
            Some("notice-coordination-unobservable") => "notice-coordination-unobservable",
            _ => "notice-attempt-not-admitted",
        },
        "notice-native-baseline-unavailable" => "notice-baseline-missing",
        code @ ("send-identity-mismatch"
        | "send-identity-unobservable"
        | "send-session-not-idle"
        | "send-idle-unobservable"
        | "send-provider-event-pending"
        | "send-completion-evidence-missing"
        | "send-guard-observation-timeout") => code,
        "notice-native-unavailable" => match reason {
            Some("baseline-unprovable") => "notice-baseline-unprovable",
            Some("authorization-timeout") => "authorization-timeout",
            _ => "notice-attempt-not-admitted",
        },
        _ => "notice-attempt-not-admitted",
    })
}
