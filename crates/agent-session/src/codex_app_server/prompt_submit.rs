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

#[derive(Debug)]
pub(super) enum PromptRequestFailure {
    Write(PromptSubmissionFailure),
    Response(PromptSubmissionFailure),
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

pub(super) async fn submit_prompt_request<S>(
    websocket: &mut S,
    id: u64,
    request: Value,
    live: Option<(&CliContext, &SessionRecord, &mut FailureReducer)>,
    external_auth: Option<(&CliContext, &SessionRecord, &str)>,
    timeout: Duration,
) -> Result<String, PromptRequestFailure>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    super::send_json(websocket, request)
        .await
        .map_err(|_| PromptRequestFailure::Write(PromptSubmissionFailure::NotConfirmed))?;
    receive_prompt_response_with_timeout(websocket, id, live, external_auth, timeout)
        .await
        .map_err(|error| match error {
            PromptResponseFailure::Rejected(_) | PromptResponseFailure::NotConfirmed(_) => {
                PromptRequestFailure::Response(PromptSubmissionFailure::NotConfirmed)
            }
            PromptResponseFailure::Other(_) => {
                PromptRequestFailure::Response(PromptSubmissionFailure::AcknowledgementUnknown)
            }
        })
        .and_then(|value| {
            value
                .pointer("/turn/id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or(PromptRequestFailure::Response(
                    PromptSubmissionFailure::NotConfirmed,
                ))
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::Role;

    async fn websocket_pair() -> (
        tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
        tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);
        let client =
            tokio_tungstenite::WebSocketStream::from_raw_socket(client_io, Role::Client, None)
                .await;
        let server =
            tokio_tungstenite::WebSocketStream::from_raw_socket(server_io, Role::Server, None)
                .await;
        (client, server)
    }

    fn prompt_request() -> Value {
        json!({ "id": 17, "method": "turn/start", "params": {} })
    }

    fn assert_response_failure(
        result: Result<String, PromptRequestFailure>,
        expected: fn(&PromptSubmissionFailure) -> bool,
    ) {
        let Err(PromptRequestFailure::Response(failure)) = result else {
            panic!("expected a response-stage failure");
        };
        assert!(expected(&failure));
    }

    fn is_acknowledgement_unknown(failure: &PromptSubmissionFailure) -> bool {
        matches!(failure, PromptSubmissionFailure::AcknowledgementUnknown)
    }

    #[tokio::test]
    async fn failed_prompt_write_is_not_confirmed() {
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);
        drop(server_io);
        let mut client =
            tokio_tungstenite::WebSocketStream::from_raw_socket(client_io, Role::Client, None)
                .await;

        let result = submit_prompt_request(
            &mut client,
            17,
            prompt_request(),
            None,
            None,
            Duration::from_millis(20),
        )
        .await;

        assert!(matches!(
            result,
            Err(PromptRequestFailure::Write(
                PromptSubmissionFailure::NotConfirmed
            ))
        ));
    }

    #[tokio::test]
    async fn protocol_timeout_after_prompt_write_is_acknowledgement_unknown() {
        let (mut client, mut server) = websocket_pair().await;
        let (observed_tx, observed_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let peer = tokio::spawn(async move {
            let request = server.next().await.unwrap().unwrap();
            assert!(request.to_text().unwrap().contains("turn/start"));
            let _ = observed_tx.send(());
            let _ = release_rx.await;
        });

        let result = submit_prompt_request(
            &mut client,
            17,
            prompt_request(),
            None,
            None,
            Duration::from_millis(20),
        )
        .await;

        observed_rx.await.unwrap();
        let _ = release_tx.send(());
        peer.await.unwrap();
        assert_response_failure(result, is_acknowledgement_unknown);
    }

    #[tokio::test]
    async fn protocol_close_after_prompt_write_is_acknowledgement_unknown() {
        let (mut client, mut server) = websocket_pair().await;
        let peer = tokio::spawn(async move {
            let request = server.next().await.unwrap().unwrap();
            assert!(request.to_text().unwrap().contains("turn/start"));
            server.send(Message::Close(None)).await.unwrap();
        });

        let result = submit_prompt_request(
            &mut client,
            17,
            prompt_request(),
            None,
            None,
            Duration::from_secs(1),
        )
        .await;

        peer.await.unwrap();
        assert_response_failure(result, is_acknowledgement_unknown);
    }

    #[tokio::test]
    async fn control_response_channel_close_is_not_confirmed() {
        let (handle, mut commands, _ready) = super::super::starting_control_channel();
        let submit = tokio::spawn(async move { handle.submit_prompt("fixture").await });
        let ControlCommand::Prompt { response, .. } = commands.recv().await.unwrap() else {
            panic!("expected a prompt command");
        };
        drop(response);

        assert!(matches!(
            submit.await.unwrap(),
            Err(PromptSubmissionFailure::NotConfirmed)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn control_response_channel_timeout_is_not_confirmed() {
        let (handle, mut commands, _ready) = super::super::starting_control_channel();
        let submit = tokio::spawn(async move { handle.submit_prompt("fixture").await });
        let ControlCommand::Prompt { response, .. } = commands.recv().await.unwrap() else {
            panic!("expected a prompt command");
        };
        tokio::task::yield_now().await;
        tokio::time::advance(CONTROL_SUBMIT_TOTAL_TIMEOUT + Duration::from_millis(1)).await;

        assert!(matches!(
            submit.await.unwrap(),
            Err(PromptSubmissionFailure::NotConfirmed)
        ));
        drop(response);
    }
}
