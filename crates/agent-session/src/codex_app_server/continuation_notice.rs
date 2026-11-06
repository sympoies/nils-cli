//! Private scheduler admission transfers the record lock through the native write.
use super::*;

impl ControlHandle {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn submit_notice(
        &self,
        message: &str,
        client_message_id: &str,
        expected: &SessionRecord,
        record_lock: crate::SessionRecordLock,
        tmux_bin: &Path,
        recovery: crate::auto_resume::notice::Recovery,
        deadline: Instant,
    ) -> Result<String, String> {
        // Pending provider ingress may need the record lock before the queued
        // command can run. The durable scheduler generation survives the gap;
        // the receiver reacquires and revalidates immediately before writing.
        drop(record_lock);
        let (response, receive) = oneshot::channel();
        let deadline_at = tokio::time::Instant::from_std(deadline);
        tokio::time::timeout_at(
            deadline_at,
            self.sender.send(ControlCommand::NoticeContinue {
                message: message.into(),
                client_message_id: client_message_id.into(),
                expected: Box::new(expected.clone()),
                tmux_bin: tmux_bin.to_owned(),
                recovery,
                deadline,
                response,
            }),
        )
        .await
        .map_err(|_| "notice enqueue expired")?
        .map_err(|_| "notice control unavailable")?;
        tokio::time::timeout_at(deadline_at, receive)
            .await
            .map_err(|_| "notice acknowledgment expired")?
            .map_err(|_| "notice control closed".to_string())?
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch<S>(
    socket: &mut S,
    request_id: u64,
    context: &CliContext,
    record: &SessionRecord,
    thread_id: &str,
    reducer: &mut FailureReducer,
    external_auth: Option<(&str, &str)>,
    message: &str,
    client: &str,
    expected: &SessionRecord,
    tmux_bin: &Path,
    recovery: &crate::auto_resume::notice::Recovery,
    deadline: Instant,
) -> Result<String, String>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let refused = || "scheduler notice boundary changed".to_string();
    if Instant::now() >= deadline || crate::notice::parse_notice(message).is_none() {
        return Err(refused());
    }
    let lock_context = context.clone();
    let lock_id = expected.id.clone();
    let record_lock = tokio::task::spawn_blocking(move || {
        crate::acquire_session_record_lock_timed(
            &lock_context,
            &lock_id,
            deadline.saturating_duration_since(Instant::now()),
        )
    })
    .await
    .map_err(|_| refused())?
    .map_err(|_| refused())?;
    let current = crate::load_session_record(context, &expected.id).map_err(|_| refused())?;
    let expected_identity = crate::serve::prompt_receipts::native_identity(expected);
    if expected_identity.is_none()
        || crate::serve::prompt_receipts::native_identity(&current) != expected_identity
        || current
            .provider_resume
            .as_ref()
            .map(|provider| provider.session_id.as_str())
            != Some(thread_id)
        || !crate::auto_resume::notice::matches(context, &current, recovery)
    {
        return Err(refused());
    }
    if let Some(active) = reducer.active_turn_id.as_deref() {
        let launch = current.runtime.as_ref().ok_or_else(refused)?;
        let projected = crate::activity::projected_codex_turn_identifier(&launch.launch_id, active)
            .map_err(|_| refused())?;
        if recovery.cause != "provider_capacity"
            || projected != recovery.predecessor
            || (reducer.pending_turns.get(active)
                != Some(&Some(StructuredFailureKind::ProviderCapacity))
                && !reducer.completed_turns.contains(active))
        {
            return Err(refused());
        }
    }
    control_account_ready(context, record, external_auth.map(|(account, _)| account)).await?;
    // TUI draft typing need not change any durable identity/recovery fence.
    // Re-observe after the queue handoff, under the write's record lock/budget.
    let observe_context = context.clone();
    let observe_record = current.clone();
    let observe_tmux = tmux_bin.to_owned();
    tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        tokio::task::spawn_blocking(move || {
            let guards = crate::send_guard::GuardArgs {
                notice_envelope: true,
                require_empty_composer: true,
                expect_runtime: observe_record
                    .runtime
                    .as_ref()
                    .map(|runtime| runtime.launch_id.clone()),
                expect_provider_session: observe_record
                    .provider_resume
                    .as_ref()
                    .map(|provider| provider.session_id.clone()),
                ..Default::default()
            };
            crate::send_guard::Admission {
                context: &observe_context,
                record: &observe_record,
                args: &guards,
                tmux_bin: &observe_tmux,
                verify_composer: true,
                deadline,
            }
            .check_preserving_draft("claimed")
            .map_err(|error| error.code().to_owned())
        }),
    )
    .await
    .map_err(|_| "notice composer observation expired".to_string())?
    .map_err(|_| "notice composer observation worker failed".to_string())??;
    if Instant::now() >= deadline {
        return Err(refused());
    }
    // The reacquired lock is held through the one write, then released before ingress can
    // synchronously checkpoint activity. No manual cancellation is introduced.
    let request = control::prompt_request(request_id, thread_id, message, Some(client));
    tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        send_json(socket, request),
    )
    .await
    .map_err(|_| "notice write expired")??;
    drop(record_lock);
    let result = receive_response_with_timeout(
        socket,
        request_id,
        Some((context, record, reducer)),
        external_auth.map(|(account, connection)| (context, record, account, connection)),
        deadline.saturating_duration_since(Instant::now()),
    )
    .await?;
    let turn = result
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .filter(|id| protocol_id_is_valid(id))
        .ok_or_else(refused)?
        .to_owned();
    reducer.note_started(&turn);
    Ok(turn)
}
