use super::*;

#[derive(Debug)]
pub(super) enum PromptResponseFailure {
    Rejected(String),
    NotConfirmed(String),
    Other(String),
}

impl PromptResponseFailure {
    pub(super) fn into_string(self) -> String {
        match self {
            Self::Rejected(error) | Self::NotConfirmed(error) | Self::Other(error) => error,
        }
    }
}

#[derive(Debug)]
pub(crate) enum PromptSubmissionFailure {
    NotDispatched,
    NotConfirmed,
    AcknowledgementUnknown,
}

#[cfg(test)]
pub(crate) fn not_confirmed(error: impl Into<String>) -> PromptSubmissionFailure {
    let _ = error.into();
    PromptSubmissionFailure::NotConfirmed
}

#[cfg(test)]
pub(crate) fn acknowledgement_unknown(error: impl Into<String>) -> PromptSubmissionFailure {
    let _ = error.into();
    PromptSubmissionFailure::AcknowledgementUnknown
}

#[cfg(test)]
pub(crate) fn not_dispatched(error: impl Into<String>) -> PromptSubmissionFailure {
    let _ = error.into();
    PromptSubmissionFailure::NotDispatched
}

impl ControlHandle {
    pub(crate) async fn submit_prompt(
        &self,
        message: &str,
    ) -> Result<String, PromptSubmissionFailure> {
        let (response, receive) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + CONTROL_SUBMIT_TOTAL_TIMEOUT;
        tokio::time::timeout_at(
            deadline,
            self.sender.send(ControlCommand::Prompt {
                message: message.to_string(),
                response,
            }),
        )
        .await
        .map_err(|_| PromptSubmissionFailure::NotDispatched)?
        .map_err(|_| PromptSubmissionFailure::NotDispatched)?;
        tokio::time::timeout_at(deadline, receive)
            .await
            .map_err(|_| PromptSubmissionFailure::NotConfirmed)?
            .map_err(|_| PromptSubmissionFailure::NotConfirmed)?
    }
}

pub(super) async fn receive_prompt_response_with_timeout<S>(
    websocket: &mut S,
    id: u64,
    live: Option<(&CliContext, &SessionRecord, &mut FailureReducer)>,
    external_auth: Option<(&CliContext, &SessionRecord, &str)>,
    timeout: Duration,
) -> Result<Value, PromptResponseFailure>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    tokio::time::timeout(
        timeout,
        receive_response(websocket, id, live, external_auth),
    )
    .await
    .map_err(|_| PromptResponseFailure::Other("Codex app-server request timed out".to_string()))?
}

pub(super) async fn receive_response<S>(
    websocket: &mut S,
    id: u64,
    mut live: Option<(&CliContext, &SessionRecord, &mut FailureReducer)>,
    external_auth: Option<(&CliContext, &SessionRecord, &str)>,
) -> Result<Value, PromptResponseFailure>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    loop {
        let value = super::decode_message(websocket.next().await)
            .await
            .map_err(PromptResponseFailure::Other)?;
        if super::respond_to_external_auth_refresh(websocket, &value, external_auth)
            .await
            .map_err(PromptResponseFailure::Other)?
        {
            continue;
        }
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            if let Some(error) = value.get("error") {
                let category = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(super::protocol_error_category)
                    .unwrap_or("unknown");
                return Err(PromptResponseFailure::Rejected(format!(
                    "Codex app-server rejected request: {} ({category})",
                    error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or_default()
                )));
            }
            return value.get("result").cloned().ok_or_else(|| {
                PromptResponseFailure::NotConfirmed(
                    "Codex app-server response omitted result".to_string(),
                )
            });
        }
        if let Some((context, record, reducer)) = live.as_mut() {
            super::process_live_message(context, record, reducer, None, &value)
                .await
                .map_err(PromptResponseFailure::Other)?;
        }
    }
}
