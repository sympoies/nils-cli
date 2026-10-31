//! Terminal/native notices reuse guarded send without a second prompt route.
use super::*;

/// Called by guarded send while it holds the recipient record lock, after
/// identity/idle/composer admission and before releasing any notice text.
pub(crate) fn begin_mailbox_notice(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    candidate: &crate::coordination::NotificationCandidate,
) -> Result<(), CliError> {
    let refused = || {
        CliError::unavailable(
            "mailbox-notice-not-ready",
            "mailbox notice eligibility changed",
            None,
        )
    };
    let agent = AgentKind::from_name(&record.agent).ok_or_else(refused)?;
    if !candidate.is_notice()
        || record.id != candidate.target_session_id
        || record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str())
            != Some(candidate.target_incarnation.as_str())
        || record.coordination_mode == cli::CoordinationMode::Off
        || !terminal_notification_waiting(context, record, agent)
        || tmux_session_attached(tmux, &record.tmux_session) != Ok(false)
        || (agent == AgentKind::Claude
            && crate::send::claude_notice::runtime::readiness(context, record).is_none())
    {
        return Err(refused());
    }
    let mut guard = crate::coordination::lock_session_quiescence(
        context,
        &candidate.target_session_id,
        &candidate.target_incarnation,
    )?;
    if !guard.begin_notification_attempt(candidate)? {
        return Err(refused());
    }
    Ok(())
}

pub(super) async fn dispatch(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
) {
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
            notice_envelope: true,
            require_idle: true,
            require_empty_composer: true,
            expect_runtime: Some(sending.target_incarnation.clone()),
            expect_provider_session: record.provider_resume.map(|provider| provider.session_id),
            idempotency_key: Some(format!(
                "mailbox-{}-{}",
                sending.target_incarnation, sending.generation
            )),
        };
        crate::send::send_mailbox_notice(&context, args, &sending)
    })
    .await;
    let context = state.context.clone();
    if matches!(result, Ok(Ok(_))) {
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
            update_notification_deferred(
                &state,
                &candidate,
                "provider-not-ready",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
        }
    }
}
