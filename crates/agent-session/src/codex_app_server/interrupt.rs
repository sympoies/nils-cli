//! Owner-only interruption on the existing app-server control connection.
use super::*;

pub(crate) const MINIMUM_CODEX_VERSION: &str = "0.151.0";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    NoActiveTurn,
    Interrupted,
    Completed,
    Failed,
}

impl Outcome {
    fn from_status(value: &Value) -> Option<Self> {
        match value.get("status")?.as_str()? {
            "interrupted" => Some(Self::Interrupted),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Busy,
    TargetChanged,
    Unsupported,
    Unavailable,
    OutcomeUnknown,
}

pub(super) type Reply = oneshot::Sender<Result<Outcome, Error>>;

impl ControlHandle {
    pub(crate) async fn interrupt(&self, thread_id: String) -> Result<Outcome, Error> {
        let (response, receive) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + CONTROL_SUBMIT_TOTAL_TIMEOUT;
        tokio::time::timeout_at(
            deadline,
            self.sender.send(ControlCommand::Interrupt {
                thread_id,
                response,
            }),
        )
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::Unavailable)?;
        tokio::time::timeout_at(deadline, receive)
            .await
            .map_err(|_| Error::OutcomeUnknown)?
            .map_err(|_| Error::Unavailable)?
    }
}

pub(super) struct Client<'a> {
    pub(super) websocket: &'a mut tokio_tungstenite::WebSocketStream<UnixStream>,
    pub(super) request_id: &'a mut u64,
    pub(super) context: &'a CliContext,
    pub(super) record: &'a SessionRecord,
    pub(super) reducer: &'a mut FailureReducer,
    pub(super) external_auth: Option<(&'a CliContext, &'a SessionRecord, &'a str, &'a str)>,
}

enum Evidence {
    Response(Value),
    Terminal(Outcome),
}

impl Client<'_> {
    pub(super) async fn dispatch(
        &mut self,
        thread_id: &str,
        reply: &Reply,
    ) -> Result<Outcome, Error> {
        if reply.is_closed() || thread_id != self.reducer.thread_id {
            return Err(Error::TargetChanged);
        }
        let lock = self.lock_target(thread_id).await?;
        // The same gate fences TUI turn starts and account/conversation mutation.
        // Keep it through settlement, but release the record lock for provider ingress.
        let _turn_start_gate = acquire_account_mutation_gate(self.context, &self.record.id)
            .map_err(|err| match err.code() {
                "codex-account-session-busy" => Error::Busy,
                _ => Error::Unavailable,
            })?;
        drop(lock);

        let id = self.next_id();
        let Evidence::Response(snapshot) =
            self.query(latest_turn_request(id, thread_id), None).await?
        else {
            unreachable!("no interrupt target was selected");
        };
        if method_missing(&snapshot) {
            return Err(Error::Unsupported);
        }
        let turn_id = match snapshot.get("result").and_then(latest_turn_state) {
            Some(LatestTurnState::Idle(_)) => return Ok(Outcome::NoActiveTurn),
            Some(LatestTurnState::InProgress(turn_id)) => turn_id,
            None if thread_not_materialized(&snapshot, thread_id) => {
                return self.confirm_fresh_idle(thread_id).await;
            }
            None => return Err(Error::Unavailable),
        };

        let id = self.next_id();
        let lock = self.lock_target(thread_id).await?;
        if reply.is_closed() {
            return Err(Error::Unavailable);
        }
        let request = json!({
            "id": id,
            "method": "turn/interrupt",
            "params": { "threadId": thread_id, "turnId": turn_id }
        });
        let sent =
            tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, send_json(self.websocket, request))
                .await;
        drop(lock);
        if !matches!(sent, Ok(Ok(()))) {
            return Err(Error::OutcomeUnknown);
        }

        let response = tokio::time::timeout(
            CONTROL_RESPONSE_TIMEOUT,
            self.receive(Some(id), Some(turn_id)),
        )
        .await;
        match response {
            Ok(Ok(Evidence::Terminal(outcome))) => return Ok(outcome),
            Ok(Ok(Evidence::Response(response))) if method_missing(&response) => {
                return Err(Error::Unsupported);
            }
            Ok(Err(_)) => return Err(Error::OutcomeUnknown),
            _ => {}
        }

        // The RPC may acknowledge with an empty object or reject a turn that
        // already completed. In either case, only exact terminal evidence counts.
        let id = self.next_id();
        match self
            .query(latest_turn_request(id, thread_id), Some(turn_id))
            .await
        {
            Ok(Evidence::Terminal(outcome)) => Ok(outcome),
            Ok(Evidence::Response(response)) => {
                if let Some(outcome) = response
                    .pointer("/result/data/0")
                    .filter(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
                    .and_then(Outcome::from_status)
                {
                    return Ok(outcome);
                }
                // The notification can follow the acknowledgement and snapshot.
                match tokio::time::timeout(
                    CONTROL_RESPONSE_TIMEOUT,
                    self.receive(None, Some(turn_id)),
                )
                .await
                {
                    Ok(Ok(Evidence::Terminal(outcome))) => Ok(outcome),
                    _ => Err(Error::OutcomeUnknown),
                }
            }
            Err(_) => Err(Error::OutcomeUnknown),
        }
    }

    async fn lock_target(&self, thread_id: &str) -> Result<crate::SessionRecordLock, Error> {
        let context = self.context.clone();
        let expected = self.record.clone();
        let thread_id = thread_id.to_owned();
        tokio::task::spawn_blocking(move || {
            // Prompt submission can hold the record lock while waiting on
            // this same control queue. Refuse contention instead of waiting.
            let lock = crate::try_acquire_session_record_lock(&context, &expected.id)
                .map_err(|_| Error::Unavailable)?
                .ok_or(Error::Busy)?;
            let current = crate::load_session_record(&context, &expected.id)
                .map_err(|_| Error::TargetChanged)?;
            crate::ensure_same_session_identity(&expected, &current)
                .map_err(|_| Error::TargetChanged)?;
            if !matches!(guarded::Client::attached_thread(&current), Ok(id) if id == thread_id) {
                return Err(Error::TargetChanged);
            }
            Ok(lock)
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }

    async fn confirm_fresh_idle(&mut self, thread_id: &str) -> Result<Outcome, Error> {
        let id = self.next_id();
        let request = json!({
            "id": id, "method": "thread/read",
            "params": { "threadId": thread_id, "includeTurns": false }
        });
        match self.query(request, None).await? {
            Evidence::Response(response)
                if response
                    .pointer("/result/thread/id")
                    .and_then(Value::as_str)
                    == Some(thread_id)
                    && response
                        .pointer("/result/thread/status/type")
                        .and_then(Value::as_str)
                        == Some("idle") =>
            {
                Ok(Outcome::NoActiveTurn)
            }
            _ => Err(Error::Unavailable),
        }
    }

    fn next_id(&mut self) -> u64 {
        *self.request_id = self.request_id.saturating_add(1);
        *self.request_id
    }

    async fn query(&mut self, request: Value, turn_id: Option<&str>) -> Result<Evidence, Error> {
        let id = request["id"].as_u64();
        tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, async {
            send_json(self.websocket, request)
                .await
                .map_err(|_| Error::Unavailable)?;
            self.receive(id, turn_id).await
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }

    async fn receive(&mut self, id: Option<u64>, turn_id: Option<&str>) -> Result<Evidence, Error> {
        loop {
            let value = decode_message(self.websocket.next().await)
                .await
                .map_err(|_| Error::Unavailable)?;
            if respond_to_external_auth_refresh(self.websocket, &value, self.external_auth)
                .await
                .map_err(|_| Error::Unavailable)?
            {
                continue;
            }
            if id.is_some()
                && value.get("method").is_none()
                && value.get("id").and_then(Value::as_u64) == id
            {
                return Ok(Evidence::Response(value));
            }
            let terminal = turn_id.and_then(|turn_id| {
                terminal_notification(&value, &self.reducer.thread_id, turn_id)
            });
            let projected =
                process_live_message(self.context, self.record, self.reducer, None, &value).await;
            if let Some(outcome) = terminal {
                if projected.is_err() {
                    eprintln!("warning: Codex interrupt activity projection degraded");
                }
                return Ok(Evidence::Terminal(outcome));
            }
            projected.map_err(|_| Error::Unavailable)?;
        }
    }
}

fn method_missing(response: &Value) -> bool {
    response.pointer("/error/code").and_then(Value::as_i64) == Some(-32601)
}

fn thread_not_materialized(response: &Value, thread_id: &str) -> bool {
    response.pointer("/error/message").and_then(Value::as_str)
        == Some(&format!(
            "thread {thread_id} is not materialized yet; thread/turns/list is unavailable before first user message"
        ))
}

fn terminal_notification(value: &Value, thread_id: &str, turn_id: &str) -> Option<Outcome> {
    if value.get("method").and_then(Value::as_str) != Some("turn/completed")
        || value.pointer("/params/threadId").and_then(Value::as_str) != Some(thread_id)
        || value.pointer("/params/turn/id").and_then(Value::as_str) != Some(turn_id)
    {
        return None;
    }
    value.pointer("/params/turn").and_then(Outcome::from_status)
}
