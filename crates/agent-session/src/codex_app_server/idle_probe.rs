//! Existing bounded identity recovery probe.
use super::*;
#[path = "startup_recovery.rs"]
mod startup_recovery;
pub(crate) use startup_recovery::verify_startup_quiescent;

/// Resolve only an unambiguous loaded primary conversation and require the
/// provider's own idle status. Never infer the active thread from history order.
pub(crate) fn probe_idle_conversation(
    context: &CliContext,
    record: &SessionRecord,
    candidate: Option<&str>,
) -> Result<String, CliError> {
    probe_idle_conversation_until(context, record, candidate, None)
}

pub(crate) fn probe_idle_conversation_until(
    context: &CliContext,
    record: &SessionRecord,
    candidate: Option<&str>,
    deadline: Option<Instant>,
) -> Result<String, CliError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            CliError::runtime(
                "conversation-probe-unavailable",
                "provider probe runtime is unavailable",
                None,
            )
        })?;
    let probe = async {
        let socket = socket_path(record)
            .ok_or_else(|| "managed Codex app-server is unavailable".to_string())?;
        let stream = connect_socket(Path::new(socket)).await?;
        let (mut websocket, _) = tokio::time::timeout(
            CONTROL_RESPONSE_TIMEOUT,
            tokio_tungstenite::client_async("ws://localhost", stream),
        )
        .await
        .map_err(|_| "provider handshake timed out".to_string())?
        .map_err(|_| "provider handshake failed".to_string())?;
        send_json(&mut websocket, initialize_request(1)).await?;
        receive_response_with_timeout(&mut websocket, 1, None, None, CONTROL_RESPONSE_TIMEOUT)
            .await?;
        send_json(&mut websocket, initialized_notification()).await?;
        send_json(&mut websocket, loaded_threads_request(2)).await?;
        let result =
            receive_response_with_timeout(&mut websocket, 2, None, None, CONTROL_RESPONSE_TIMEOUT)
                .await?;
        if result
            .get("nextCursor")
            .is_some_and(|cursor| !cursor.is_null())
        {
            return Err("loaded conversation identities are incomplete".to_string());
        }
        let ids = loaded_thread_ids(&result)
            .ok_or_else(|| "loaded conversation identities were invalid".to_string())?;
        let mut primary = Vec::new();
        for id in ids {
            if !system_ephemeral_raw_session_is_registered(context, record, &id)
                .map_err(|_| "auxiliary identity classification failed".to_string())?
            {
                primary.push(id);
            }
        }
        let thread_id = match candidate {
            Some(id) if primary.iter().any(|value| value == id) => id.to_string(),
            None if primary.len() == 1 => primary.remove(0),
            _ => {
                return Err(
                    "live primary conversation cannot be identified unambiguously".to_string(),
                );
            }
        };
        send_json(&mut websocket, json!({"id":3,"method":"thread/read","params":{"threadId":thread_id,"includeTurns":false}})).await?;
        let result =
            receive_response_with_timeout(&mut websocket, 3, None, None, CONTROL_RESPONSE_TIMEOUT)
                .await?;
        if result.pointer("/thread/id").and_then(Value::as_str) != Some(thread_id.as_str())
            || result
                .pointer("/thread/status/type")
                .and_then(Value::as_str)
                != Some("idle")
        {
            return Err("live provider conversation is not verified idle".to_string());
        }
        Ok(thread_id)
    };
    runtime.block_on(async {
        if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline.into(), probe)
                .await
                .map_err(|_| "provider idle probe timed out".to_string())?
        } else {
            probe.await
        }
    }).map_err(|_| CliError::runtime("conversation-live-identity-unverified", "live Codex conversation is busy, ambiguous, or unavailable; no binding was changed", Some(json!({"id":record.id}))))
}

/// Called only after a successful primary thread/start response has been bound,
/// and before it reaches the TUI. Startup readiness does not finish a turn.
pub(super) async fn settle_startup(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), String> {
    let context = context.clone();
    let record = record.clone();
    tokio::task::spawn_blocking(move || {
        let Some(state) = crate::activity::controller_state_for_record(&context, &record) else {
            return Ok(false);
        };
        crate::activity::settle_starting(&context, &record, Some(state.revision))
    })
    .await
    .map_err(|_| "Codex startup observation worker failed".to_string())?
    .map_err(|error: CliError| format!("Codex startup observation failed: {}", error.code()))?;
    Ok(())
}

/// Recover resumed startup from an exact-thread snapshot on the subscribed control stream.
pub(super) async fn recover_resumed<S>(
    websocket: &mut S,
    request_id: &mut u64,
    context: &CliContext,
    record: &SessionRecord,
    reducer: &mut FailureReducer,
    external_auth: Option<(&CliContext, &SessionRecord, &str, &str)>,
) where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    for _ in 0..3 {
        let Some(state) = crate::activity::controller_state_for_record(context, record) else {
            return;
        };
        if state.phase != crate::activity::TurnPhase::Starting || reducer.active_turn_id.is_some() {
            return;
        }
        reducer.project_resumed_starts = true;
        *request_id = request_id.saturating_add(1);
        let request = json!({"id": *request_id, "method":"thread/read", "params": {
            "threadId": reducer.thread_id, "includeTurns": false
        }});
        if !matches!(
            tokio::time::timeout(Duration::from_secs(2), send_json(websocket, request)).await,
            Ok(Ok(()))
        ) {
            return;
        }
        let result = receive_response_with_timeout(
            websocket,
            *request_id,
            Some((context, record, reducer)),
            external_auth,
            Duration::from_secs(2),
        )
        .await;
        if let Ok(result) = result {
            if result.pointer("/thread/id").and_then(Value::as_str)
                == Some(reducer.thread_id.as_str())
                && result
                    .pointer("/thread/status/type")
                    .and_then(Value::as_str)
                    == Some("idle")
                && reducer.active_turn_id.is_none()
            {
                let context = context.clone();
                let record = record.clone();
                let revision = state.revision;
                if matches!(
                    tokio::task::spawn_blocking(move || {
                        crate::activity::state_view::settle_resumed_starting(
                            &context, &record, revision,
                        )
                    })
                    .await,
                    Ok(Ok(true))
                ) {
                    return;
                }
            }
            if result
                .pointer("/thread/status/type")
                .and_then(Value::as_str)
                == Some("active")
            {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) fn latest_turn_request(id: u64, thread_id: &str) -> Value {
    json!({
        "id": id,
        "method": "thread/turns/list",
        "params": {
            "threadId": thread_id,
            "limit": 1,
            "sortDirection": "desc",
            "itemsView": "notLoaded"
        }
    })
}

/// Rejoin the bound thread for live control without hydrating its unbounded
/// turn history into a single WebSocket response.
pub(crate) fn resume_thread_request(id: u64, thread_id: &str, cwd: &str) -> Value {
    json!({
        "id": id,
        "method": "thread/resume",
        "params": { "threadId": thread_id, "cwd": cwd, "excludeTurns": true }
    })
}
