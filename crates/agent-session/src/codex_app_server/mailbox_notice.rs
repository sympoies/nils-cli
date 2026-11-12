//! Mailbox queue handoff revalidates its consumed generation at the native write.
use super::*;

pub(crate) struct Write {
    pub(crate) expected: Box<SessionRecord>,
    pub(crate) candidate: crate::coordination::NotificationCandidate,
    pub(crate) expected_turn: Option<String>,
    pub(crate) deadline: Instant,
}

pub(crate) struct WriteGuard {
    _record: crate::SessionRecordLock,
    _quiescence: crate::coordination::SessionQuiescenceGuard,
    pub(super) deadline: Instant,
}

pub(super) async fn lock(
    context: &CliContext,
    thread_id: &str,
    text: &str,
    write: Option<&Write>,
) -> Result<Option<WriteGuard>, String> {
    let Some(write) = write else { return Ok(None) };
    let context = context.clone();
    let thread_id = thread_id.to_owned();
    let text = text.to_owned();
    let expected = write.expected.clone();
    let candidate = write.candidate.clone();
    let expected_turn = write.expected_turn.clone();
    let deadline = write.deadline;
    tokio::task::spawn_blocking(move || {
        let refused = || "mailbox notice boundary changed".to_string();
        let record_lock = crate::acquire_session_record_lock_timed(
            &context,
            &expected.id,
            deadline.saturating_duration_since(Instant::now()),
        )
        .map_err(|_| refused())?;
        let current = crate::load_session_record(&context, &expected.id).map_err(|_| refused())?;
        let identity = crate::serve::prompt_receipts::native_identity(&expected);
        if Instant::now() >= deadline
            || !crate::notice::writers_enabled()
            || !candidate.is_notice()
            || candidate.prompt.as_deref() != Some(text.as_str())
            || identity.is_none()
            || crate::serve::prompt_receipts::native_identity(&current) != identity
            || !runtime_is_supported(&current)
            || current
                .provider_resume
                .as_ref()
                .map(|provider| provider.session_id.as_str())
                != Some(thread_id.as_str())
        {
            return Err(refused());
        }
        let eligible = crate::activity::state_for_view(&context, &current).is_some_and(|turn| {
            match expected_turn.as_deref() {
                Some(expected) => {
                    matches!(
                        turn.phase,
                        crate::activity::TurnPhase::Working
                            | crate::activity::TurnPhase::NeedsInput
                    ) && turn.source.confidence == crate::activity::Confidence::Authoritative
                        && turn
                            .current_turn
                            .as_ref()
                            .and_then(|turn| turn.provider_turn_id.as_deref())
                            == Some(expected)
                }
                None => crate::activity::codex_notification_maybe_idle(&turn),
            }
        });
        let quiescence = crate::coordination::lock_session_quiescence(
            &context,
            &candidate.target_session_id,
            &candidate.target_incarnation,
        )
        .map_err(|_| refused())?;
        if !eligible
            || quiescence
                .notification_block(expected_turn.is_some())
                .is_some()
            || !quiescence.notice_attempt_matches(&candidate)
        {
            return Err(refused());
        }
        Ok(Some(WriteGuard {
            _record: record_lock,
            _quiescence: quiescence,
            deadline,
        }))
    })
    .await
    .map_err(|_| "mailbox notice admission worker failed".to_string())?
}

pub(super) async fn send<S>(
    socket: &mut S,
    request: Value,
    guard: Option<WriteGuard>,
) -> Result<(), String>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let result = if let Some(guard) = guard.as_ref() {
        if Instant::now() >= guard.deadline {
            return Err("mailbox notice write expired".to_string());
        }
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(guard.deadline),
            send_json(socket, request),
        )
        .await
        .map_err(|_| "mailbox notice write expired".to_string())?
    } else {
        send_json(socket, request).await
    };
    // Provider ingress can synchronously checkpoint activity only after this write.
    drop(guard);
    result
}

impl ControlHandle {
    pub(crate) async fn mailbox_prompt(
        &self,
        message: &str,
        client: &str,
        write: Write,
    ) -> Result<String, prompt_submit::PromptSubmissionFailure> {
        let (response, receive) = oneshot::channel();
        let deadline = tokio::time::Instant::from_std(write.deadline);
        tokio::time::timeout_at(
            deadline,
            self.sender.send(ControlCommand::Prompt {
                message: message.to_owned(),
                client_message_id: Some(client.into()),
                write: Some(Box::new(write)),
                response,
            }),
        )
        .await
        .map_err(|_| prompt_submit::PromptSubmissionFailure::NotDispatched)?
        .map_err(|_| prompt_submit::PromptSubmissionFailure::NotDispatched)?;
        tokio::time::timeout_at(deadline, receive)
            .await
            .map_err(|_| prompt_submit::PromptSubmissionFailure::NotConfirmed)?
            .map_err(|_| prompt_submit::PromptSubmissionFailure::NotConfirmed)?
    }

    pub(crate) async fn steer_prompt(
        &self,
        message: &str,
        expected_turn_id: &str,
        client_message_id: Option<&str>,
        write: Option<mailbox_notice::Write>,
    ) -> Result<String, String> {
        let deadline = write
            .as_ref()
            .map(|write| tokio::time::Instant::from_std(write.deadline));
        let (response, receive) = oneshot::channel();
        let enqueue_deadline = tokio::time::Instant::now() + CONTROL_RESPONSE_TIMEOUT;
        let enqueue_deadline =
            deadline.map_or(enqueue_deadline, |deadline| deadline.min(enqueue_deadline));
        tokio::time::timeout_at(
            enqueue_deadline,
            self.sender.send(ControlCommand::Steer {
                message: message.to_string(),
                expected_turn_id: expected_turn_id.to_string(),
                client_message_id: client_message_id.map(str::to_owned),
                write: write.map(Box::new),
                response,
            }),
        )
        .await
        .map_err(|_| "codex turn steering enqueue timed out".to_string())?
        .map_err(|_| "codex control connection unavailable".to_string())?;
        // A notice shares its original deadline with the receiver's write fence.
        // Unfenced native callers wait for the response or connection close.
        if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline, receive)
                .await
                .map_err(|_| "mailbox notice acknowledgment expired".to_string())?
                .map_err(|_| "codex control connection closed".to_string())?
        } else {
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        }
    }
}
