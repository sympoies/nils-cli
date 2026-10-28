//! Observe startup quiescence without inventing or attaching a conversation identity.
use super::*;

pub(crate) fn verify_startup_quiescent(
    record: &SessionRecord,
    deadline: Instant,
) -> Result<(), &'static str> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "send-idle-unobservable")?;
    runtime.block_on(async {
        tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), async {
            let socket = socket_path(record).ok_or("socket unavailable".to_string())?;
            let stream = connect_socket(Path::new(socket)).await?;
            let (mut websocket, _) = tokio_tungstenite::client_async("ws://localhost", stream).await.map_err(|_| "handshake failed".to_string())?;
            send_json(&mut websocket, initialize_request(1)).await?;
            receive_response_with_timeout(&mut websocket, 1, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
            send_json(&mut websocket, initialized_notification()).await?;
            send_json(&mut websocket, loaded_threads_request(2)).await?;
            let result = receive_response_with_timeout(&mut websocket, 2, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
            let ids = loaded_thread_ids(&result).ok_or("invalid thread list".to_string())?;
            if result.get("nextCursor").is_some_and(|cursor| !cursor.is_null()) || ids.len() > 1 {
                return Err("ambiguous thread list".to_string());
            }
            if let Some(id) = ids.first() {
                send_json(&mut websocket, json!({"id":3,"method":"thread/read","params":{"threadId":id,"includeTurns":false}})).await?;
                let result = receive_response_with_timeout(&mut websocket, 3, None, None, CONTROL_RESPONSE_TIMEOUT).await?;
                if result.pointer("/thread/id").and_then(Value::as_str) != Some(id.as_str())
                    || result.pointer("/thread/status/type").and_then(Value::as_str) != Some("idle") {
                    return Err("thread not idle".to_string());
                }
            }
            Ok::<_, String>(())
        }).await.map_err(|_| "send-idle-unobservable")?
            .map_err(|_| "send-session-not-idle")
    })
}
