//! Message-size limits and failure recording for the Codex app-server proxy.
//!
//! The app-server sends some responses whose size follows an external catalog
//! (the remote plugin list was 16.8 MB and growing), so the limits are layered:
//!
//! - `HARD_MESSAGE_LIMIT_BYTES` is the WebSocket ceiling on both proxy
//!   connections. The WebSocket layer cannot resynchronise after a frame above
//!   it, so exceeding it still ends the proxy. It bounds memory per message:
//!   128 MiB is roughly 7x the payload that motivated this module while a
//!   proxy holds at most one such message at a time.
//! - `MAX_FORWARDED_MESSAGE_BYTES` is the largest app-server message the proxy
//!   relays. A larger one is contained: the request it answers fails with a
//!   JSON-RPC error instead of the proxy exiting. 64 MiB is also the default
//!   message limit of tungstenite-based clients, so forwarding more would only move
//!   the failure into the TUI.

use super::*;

pub(super) const HARD_MESSAGE_LIMIT_BYTES: usize = 128 * 1024 * 1024;
const MAX_FORWARDED_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const JSON_RPC_INTERNAL_ERROR: i64 = -32603;
const MAX_EXIT_LINE_CHARS: usize = 1024;

/// How the proxy disposes of a server message above the forwarding limit.
#[derive(Debug, PartialEq)]
pub(super) enum Oversized {
    /// Answer the app-server's response by failing the TUI request.
    ToTui(Message),
    /// Reject an app-server request so the server is not left waiting.
    ToUpstream(Message),
    /// Nothing is waiting on a notification, so it is dropped.
    Drop,
}

impl Oversized {
    pub(super) async fn contain<T, U>(self, tui: &mut T, upstream: &mut U) -> Result<(), String>
    where
        T: futures_util::Sink<Message> + Unpin,
        T::Error: std::fmt::Display,
        U: futures_util::Sink<Message> + Unpin,
        U::Error: std::fmt::Display,
    {
        match self {
            Self::ToTui(reply) => tui
                .send(reply)
                .await
                .map_err(|err| format!("remote TUI write failed: {err}")),
            Self::ToUpstream(reply) => {
                send_proxy_upstream(upstream, reply, CONTROL_RESPONSE_TIMEOUT).await
            }
            Self::Drop => Ok(()),
        }
    }
}

pub(super) fn contain_oversized(message: &Message) -> Result<Option<Oversized>, String> {
    contain_over(message, MAX_FORWARDED_MESSAGE_BYTES)
}

fn contain_over(message: &Message, limit: usize) -> Result<Option<Oversized>, String> {
    let bytes = match message {
        Message::Text(text) => text.as_bytes(),
        Message::Binary(bytes) => bytes.as_ref(),
        _ => return Ok(None),
    };
    if bytes.len() <= limit {
        return Ok(None);
    }
    #[derive(Deserialize)]
    struct Envelope {
        id: Option<Value>,
        method: Option<String>,
    }
    // Serde skips the oversized `result`/`params` without materialising it.
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| "proxy observed malformed oversized JSON".to_string())?;
    let size = bytes.len();
    eprintln!(
        "warning: Codex app-server message of {size} bytes exceeds the {limit}-byte proxy forwarding limit"
    );
    let id = envelope.id.filter(|id| json_id_key(id).is_some());
    Ok(Some(match (envelope.method, id) {
        (None, Some(id)) => Oversized::ToTui(too_large_response(&id, size, limit)),
        (Some(_), Some(id)) => Oversized::ToUpstream(too_large_response(&id, size, limit)),
        _ => Oversized::Drop,
    }))
}

fn too_large_response(id: &Value, size: usize, limit: usize) -> Message {
    Message::Text(
        json!({
            "id": id,
            "error": {
                "code": JSON_RPC_INTERNAL_ERROR,
                "message": format!(
                    "app-server message of {size} bytes exceeds the {limit}-byte proxy forwarding limit"
                ),
                "data": {
                    "reason": "proxy_message_too_large",
                    "retryable": false,
                    "size_bytes": size,
                    "limit_bytes": limit
                }
            }
        })
        .to_string()
        .into(),
    )
}

pub(super) async fn send_proxy_upstream<S>(
    upstream: &mut S,
    message: Message,
    timeout: Duration,
) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    tokio::time::timeout(timeout, upstream.send(message))
        .await
        .map_err(|_| "upstream app-server write timed out".to_string())?
        .map_err(|err| format!("upstream app-server write failed: {err}"))
}

/// Keep the proxy's final error line where session diagnostics can find it.
///
/// The launcher only retains the proxy's stderr once the whole runtime has
/// exited, so a proxy that dies while the TUI keeps running would otherwise
/// leave no trace. The line is content-free and replaced by the launcher's
/// fuller stderr tail if that is retained later.
pub(super) fn record_proxy_exit(context: &CliContext, id: &str, line: &str) {
    let line: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_EXIT_LINE_CHARS)
        .collect();
    let _ = write_private_file(
        &crate::session_dir(context, id).join(crate::STARTUP_DIAGNOSTIC_FILE),
        format!("{line}\n").as_bytes(),
    );
}

#[cfg(test)]
mod tests;
