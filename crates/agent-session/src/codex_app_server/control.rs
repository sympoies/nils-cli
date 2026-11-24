//! Typed native control commands.
use super::*;

pub(crate) enum ControlCommand {
    Usage(oneshot::Sender<Result<UsageSnapshot, String>>),
    Interrupt {
        thread_id: String,
        response: interrupt::Reply,
    },
    Prompt {
        message: String,
        image_input: Option<Vec<Value>>,
        client_message_id: Option<String>,
        write: Option<Box<mailbox_notice::Write>>,
        response: oneshot::Sender<Result<String, prompt_submit::PromptSubmissionFailure>>,
    },
    Steer {
        message: String,
        expected_turn_id: String,
        client_message_id: Option<String>,
        write: Option<Box<mailbox_notice::Write>>,
        response: oneshot::Sender<Result<String, String>>,
    },
    NoticeContinue {
        message: String,
        client_message_id: String,
        expected: Box<SessionRecord>,
        tmux_bin: PathBuf,
        recovery: crate::auto_resume::notice::Recovery,
        deadline: Instant,
        response: oneshot::Sender<Result<String, String>>,
    },
    BindAccount {
        account: String,
        revision: u64,
        response: oneshot::Sender<Result<crate::codex_account::CodexAccountView, String>>,
    },
    /// Drain and apply a queued next-account intent if the turn is idle. Used by
    /// the periodic idle-boundary drive; a no-op when nothing is drainable.
    ApplyNext {
        response: oneshot::Sender<Result<(), String>>,
    },
}

pub(super) fn prompt_request(
    id: u64,
    thread: &str,
    message: &str,
    client: Option<&str>,
    image_input: Option<Vec<Value>>,
) -> Value {
    let mut request = continuation_request(id, thread, message);
    if let Some(input) = image_input {
        request["params"]["input"] = Value::Array(input);
    }
    if let Some(client) = client {
        request["params"]["clientUserMessageId"] = json!(client);
    }
    request
}

pub(super) async fn discover_attached_thread(
    websocket: &mut tokio_tungstenite::WebSocketStream<UnixStream>,
    request_id: &mut u64,
    record: &SessionRecord,
    commands: &mpsc::Receiver<ControlCommand>,
    external_auth: Option<(&CliContext, &SessionRecord, &str, &str)>,
) -> Result<Option<String>, String> {
    let attachment = thread_attached_path(record)
        .ok_or_else(|| "Codex attached marker metadata is missing".to_string())?;
    let poll = Duration::from_millis(100);
    // Codex 0.162.1 starts the TUI thread asynchronously. Wait for the proxy
    // to attest that response before spending the loaded-thread retry budget.
    if record.provider_resume.is_none() {
        while !attachment
            .try_exists()
            .map_err(|_| "Codex attached thread binding was unreadable".to_string())?
        {
            if commands.is_closed() {
                return Ok(None);
            }
            tokio::select! {
                () = tokio::time::sleep(poll) => {}
                message = websocket.next() => {
                    let value = decode_message(message).await?;
                    respond_to_external_auth_refresh(websocket, &value, external_auth).await?;
                }
            }
        }
    }

    for attempt in 0..=100 {
        *request_id = request_id.saturating_add(1);
        send_json(websocket, loaded_threads_request(*request_id)).await?;
        let result = receive_response_with_timeout(
            websocket,
            *request_id,
            None,
            external_auth,
            CONTROL_RESPONSE_TIMEOUT,
        )
        .await
        .map_err(|err| format!("thread/loaded/list failed: {err}"))?;
        let ids = loaded_thread_ids(&result)
            .ok_or_else(|| "Codex loaded-thread response was malformed".to_string())?;
        if let Some(id) = attached_loaded_thread(record, &ids)? {
            return Ok(Some(id));
        }
        if attempt < 100 {
            tokio::time::sleep(poll).await;
        }
    }

    Err("Codex app-server could not verify an attached loaded thread (codex-app-server-thread-binding-failed)".to_string())
}

pub(super) async fn receive_initial_usage(
    websocket: &mut tokio_tungstenite::WebSocketStream<UnixStream>,
    request_id: u64,
    (context, record, reducer): (&CliContext, &SessionRecord, &mut FailureReducer),
    external_auth: Option<(&CliContext, &SessionRecord, &str, &str)>,
) -> Result<Option<UsageSnapshot>, String> {
    match prompt_submit::receive_prompt_response_with_timeout(
        websocket,
        request_id,
        Some((context, record, reducer)),
        external_auth,
        CONTROL_RESPONSE_TIMEOUT,
    )
    .await
    {
        Ok(result) => Ok(Some(usage_snapshot(&result))),
        // Codex 0.162.1 refuses this read for API-key accounts.
        Err(prompt_submit::PromptResponseFailure::Rejected {
            code: -32600,
            category: "rate_limits_require_chatgpt",
        }) => Ok(None),
        Err(error) => Err(error.into_string()),
    }
}

impl ControlHandle {
    // Registration permits account recovery while startup is still in progress.
    // Prompt callers must wait without holding the record lock needed by thread
    // persistence and the initial usage wakeup.
    pub(crate) async fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        let mut ready = self.ready.clone();
        tokio::time::timeout(timeout, async {
            loop {
                if *ready.borrow_and_update() {
                    return Ok(());
                }
                ready
                    .changed()
                    .await
                    .map_err(|_| "codex control startup ended".to_string())?;
            }
        })
        .await
        .map_err(|_| "codex control startup timed out".to_string())?
    }

    pub(crate) async fn usage(&self) -> Result<UsageSnapshot, String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, async {
            self.sender
                .send(ControlCommand::Usage(response))
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "codex rate-limit request timed out".to_string())?
    }

    pub(crate) async fn bind_account(
        &self,
        account: &str,
        revision: u64,
    ) -> Result<crate::codex_account::CodexAccountView, String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_SUBMIT_TOTAL_TIMEOUT, async {
            self.sender
                .send(ControlCommand::BindAccount {
                    account: account.to_string(),
                    revision,
                    response,
                })
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "Codex account binding timed out".to_string())?
    }

    pub(crate) async fn apply_next(&self) -> Result<(), String> {
        let (response, receive) = oneshot::channel();
        tokio::time::timeout(CONTROL_SUBMIT_TOTAL_TIMEOUT, async {
            self.sender
                .send(ControlCommand::ApplyNext { response })
                .await
                .map_err(|_| "codex control connection unavailable".to_string())?;
            receive
                .await
                .map_err(|_| "codex control connection closed".to_string())?
        })
        .await
        .map_err(|_| "Codex next-account apply timed out".to_string())?
    }
}
