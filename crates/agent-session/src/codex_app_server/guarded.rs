//! Native guarded dispatch. Thread idle never substitutes for composer proof.
pub(crate) mod trace;
use super::*;
use trace::{Method, Outcome, Request};

type Socket = tokio_tungstenite::WebSocketStream<UnixStream>;

pub(crate) struct DispatchError {
    pub(crate) acknowledged: bool,
}

pub(crate) struct Client {
    runtime: tokio::runtime::Runtime,
    socket: Socket,
    thread_id: String,
    request_id: u64,
    deadline: Instant,
}

impl Client {
    pub(crate) fn connect(record: &SessionRecord, deadline: Instant) -> Result<Self, &'static str> {
        let thread_id = record
            .provider_resume
            .as_ref()
            .filter(|resume| resume.provider == "codex" && protocol_id_is_valid(&resume.session_id))
            .map(|resume| resume.session_id.clone())
            .ok_or("send-identity-unobservable")?;
        let attached = thread_attached_path(record).ok_or("send-identity-unobservable")?;
        if fs::read_to_string(attached).ok().as_deref()
            != Some(&projected_thread_binding(&thread_id))
        {
            return Err("send-identity-mismatch");
        }
        let path = socket_path(record).ok_or("send-guard-unsupported")?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "send-idle-unobservable")?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let connect = Request::start(Method::Connect, deadline, None);
        let socket = runtime.block_on(async {
            tokio::time::timeout(remaining, async {
                // One connection attempt: never retry delivery on another socket.
                let stream = UnixStream::connect(path)
                    .await
                    .map_err(|_| Outcome::Unavailable)?;
                let (mut socket, _) = tokio_tungstenite::client_async_with_config(
                    "ws://localhost",
                    stream,
                    Some(proxy_websocket_config()),
                )
                .await
                .map_err(|_| Outcome::Unavailable)?;
                let initialize = Request::start(Method::Initialize, deadline, None);
                let initialized = async {
                    send_json(&mut socket, initialize_request(1))
                        .await
                        .map_err(|_| Outcome::Unavailable)?;
                    receive_response_with_timeout(&mut socket, 1, None, None, remaining)
                        .await
                        .map_err(|error| trace::rpc_error(&error))?;
                    Ok::<_, Outcome>(())
                }
                .await;
                initialize.finish(
                    initialized
                        .as_ref()
                        .map_or_else(|error| *error, |_| Outcome::Ok),
                );
                initialized?;
                send_json(&mut socket, initialized_notification())
                    .await
                    .map_err(|_| Outcome::Unavailable)?;
                Ok::<_, Outcome>(socket)
            })
            .await
            .map_err(|_| Outcome::Timeout)?
        });
        connect.finish(socket.as_ref().map_or_else(|error| *error, |_| Outcome::Ok));
        let socket = socket.map_err(|_| "send-idle-unobservable")?;
        let mut client = Self {
            runtime,
            socket,
            thread_id,
            request_id: 1,
            deadline,
        };
        let loaded = client
            .request("thread/loaded/list", json!({}))
            .map_err(|_| "send-idle-unobservable")?;
        let ids = loaded_thread_ids(&loaded).ok_or("send-identity-unobservable")?;
        if loaded
            .get("nextCursor")
            .is_some_and(|cursor| !cursor.is_null())
            || !ids.iter().any(|id| id == &client.thread_id)
        {
            return Err("send-identity-mismatch");
        }
        Ok(client)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, ()> {
        self.request_id += 1;
        let id = self.request_id;
        let kind = match method {
            "thread/loaded/list" => Method::LoadedThreads,
            "thread/read" => Method::ThreadRead,
            "turn/start" => Method::TurnStart,
            "thread/compact/start" => Method::CompactStart,
            _ => unreachable!("fixed guarded RPC methods"),
        };
        let trace = Request::start(
            kind,
            self.deadline,
            params.get("includeTurns").and_then(Value::as_bool),
        );
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            trace.finish(Outcome::DeadlineExpired);
            return Err(());
        }
        let socket = &mut self.socket;
        let result = self.runtime.block_on(async {
            tokio::time::timeout(remaining, async {
                send_json(socket, json!({"id":id,"method":method,"params":params}))
                    .await
                    .map_err(|_| Outcome::Unavailable)?;
                receive_response_with_timeout(socket, id, None, None, remaining)
                    .await
                    .map_err(|error| trace::rpc_error(&error))
            })
            .await
            .map_err(|_| Outcome::Timeout)?
        });
        trace.finish(result.as_ref().map_or_else(|error| *error, |_| Outcome::Ok));
        result.map_err(|_| ())
    }

    pub(crate) fn idle(&mut self) -> Result<(), &'static str> {
        let result = self
            .request(
                "thread/read",
                json!({"threadId":self.thread_id,"includeTurns":false}),
            )
            .map_err(|_| "send-idle-unobservable")?;
        if result.pointer("/thread/id").and_then(Value::as_str) != Some(&self.thread_id) {
            return Err("send-identity-mismatch");
        }
        match result
            .pointer("/thread/status/type")
            .and_then(Value::as_str)
        {
            Some("idle") => Ok(()),
            Some("active") => Err("send-session-not-idle"),
            _ => Err("send-idle-unobservable"),
        }
    }

    pub(crate) fn dispatch(&mut self, text: &str) -> Result<&'static str, DispatchError> {
        let unknown = |_| DispatchError {
            acknowledged: false,
        };
        if text.trim() == "/compact" {
            let result = self
                .request("thread/compact/start", json!({"threadId":self.thread_id}))
                .map_err(unknown)?;
            // This acknowledges start, not completion; orchestration must still
            // observe a confirmed compaction before sending its next prompt.
            return result
                .as_object()
                .filter(|object| object.is_empty())
                .map(|_| "compact_started")
                .ok_or(DispatchError {
                    acknowledged: false,
                });
        }
        let result = self
            .request(
                "turn/start",
                json!({"threadId":self.thread_id,
            "input":[{"type":"text","text":text,"text_elements":[]}]}),
            )
            .map_err(unknown)?;
        let turn_id = result
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .filter(|id| protocol_id_is_valid(id));
        trace::turn_ack(turn_id.is_some());
        let turn_id = turn_id.ok_or(DispatchError {
            acknowledged: false,
        })?;
        let turn_id = turn_id.to_owned();
        self.confirm_input(&turn_id, text)
            .map_err(|_| DispatchError { acknowledged: true })?;
        Ok("input_observed")
    }

    fn confirm_input(&mut self, turn_id: &str, text: &str) -> Result<(), ()> {
        // The WebSocket frame/message cap bounds history hydration. Unknown,
        // oversized or unavailable history is unconfirmed, never a resend.
        loop {
            let history = self.request(
                "thread/read",
                json!({"threadId":self.thread_id,"includeTurns":true}),
            )?;
            let observed = input_observed(&history, &self.thread_id, turn_id, text);
            trace::observation(&observed);
            if observed.result? {
                return Ok(());
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(());
            }
            // Poll observation only. The mutation is never repeated.
            thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}

fn input_observed(
    history: &Value,
    thread_id: &str,
    turn_id: &str,
    text: &str,
) -> trace::Observation {
    let mut observed = trace::Observation::invalid();
    observed.thread_matches =
        Some(history.pointer("/thread/id").and_then(Value::as_str) == Some(thread_id));
    if observed.thread_matches != Some(true) {
        return observed;
    }
    let Some(turns) = history.pointer("/thread/turns").and_then(Value::as_array) else {
        return observed;
    };
    let mut matching = turns
        .iter()
        .filter(|turn| turn["id"].as_str() == Some(turn_id));
    let Some(turn) = matching.next() else {
        observed.turn_matches = Some(false);
        observed.result = Ok(false);
        return observed;
    };
    observed.turn_matches = Some(matching.next().is_none());
    if observed.turn_matches != Some(true) {
        return observed;
    }
    let Some(items) = turn["items"].as_array() else {
        return observed;
    };
    let mut inputs = items.iter().filter(|item| item["type"] == "userMessage");
    let Some(input) = inputs.next() else {
        observed.result = Ok(false);
        return observed;
    };
    if inputs.next().is_some() {
        return observed;
    }
    let Some(content) = input["content"].as_array() else {
        return observed;
    };
    let exact = content.len() == 1
        && content[0]["type"] == "text"
        && content[0]["text"].as_str() == Some(text);
    observed.text_matches = Some(exact);
    if exact {
        observed.result = Ok(true);
    }
    observed
}

pub(crate) fn idle(record: &SessionRecord, deadline: Instant) -> Result<(), &'static str> {
    Client::connect(record, deadline)?.idle()
}
