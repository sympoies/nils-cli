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
    runtime.block_on(async {
        let socket = socket_path(record).ok_or_else(|| "managed Codex app-server is unavailable".to_string())?;
        let stream = connect_socket(Path::new(socket)).await?;
        let (mut websocket, _) = tokio::time::timeout(CONTROL_RESPONSE_TIMEOUT, tokio_tungstenite::client_async("ws://localhost", stream))
            .await.map_err(|_| "provider handshake timed out".to_string())?
            .map_err(|_| "provider handshake failed".to_string())?;
        send_json(&mut websocket, initialize_request(1)).await?;
        receive_response_with_timeout(&mut websocket, 1, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
        send_json(&mut websocket, initialized_notification()).await?;
        send_json(&mut websocket, loaded_threads_request(2)).await?;
        let result = receive_response_with_timeout(&mut websocket, 2, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
        if result.get("nextCursor").is_some_and(|cursor| !cursor.is_null()) {
            return Err("loaded conversation identities are incomplete".to_string());
        }
        let ids = loaded_thread_ids(&result).ok_or_else(|| "loaded conversation identities were invalid".to_string())?;
        let mut primary = Vec::new();
        for id in ids {
            if !system_ephemeral_raw_session_is_registered(context, record, &id).map_err(|_| "auxiliary identity classification failed".to_string())? { primary.push(id); }
        }
        let thread_id = match candidate {
            Some(id) if primary.iter().any(|value| value == id) => id.to_string(),
            None if primary.len() == 1 => primary.remove(0),
            _ => return Err("live primary conversation cannot be identified unambiguously".to_string()),
        };
        send_json(&mut websocket, json!({"id":3,"method":"thread/read","params":{"threadId":thread_id,"includeTurns":false}})).await?;
        let result = receive_response_with_timeout(&mut websocket, 3, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
        if result.pointer("/thread/id").and_then(Value::as_str) != Some(thread_id.as_str())
            || result.pointer("/thread/status/type").and_then(Value::as_str) != Some("idle") {
            return Err("live provider conversation is not verified idle".to_string());
        }
        Ok(thread_id)
    }).map_err(|_| CliError::runtime("conversation-live-identity-unverified", "live Codex conversation is busy, ambiguous, or unavailable; no binding was changed", Some(json!({"id":record.id}))))
}
