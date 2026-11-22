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
