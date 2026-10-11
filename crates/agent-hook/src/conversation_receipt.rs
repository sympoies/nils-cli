//! Durable private native-clear evidence, independent of helper availability.
use std::fs;

use serde_json::{Value, json};

use crate::{error::HookError, paths};

pub(crate) fn retain(session: &str, runtime: &str, receipt: &[u8]) -> Result<(), HookError> {
    let unavailable = || {
        HookError::runtime(
            "conversation-receipt-unavailable",
            "native clear recovery receipt could not be retained",
        )
    };
    if session.is_empty()
        || !session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(unavailable());
    }
    let dir = paths::agent_session_state_root()?
        .join("sessions")
        .join(session);
    paths::ensure_private_state_dir(&dir, "conversation-receipt")?;
    let record: Value =
        serde_json::from_slice(&fs::read(dir.join("session.json")).map_err(|_| unavailable())?)
            .map_err(|_| unavailable())?;
    let generation = record
        .pointer("/runtime/generation")
        .and_then(Value::as_u64)
        .ok_or_else(unavailable)?;
    if record.get("id").and_then(Value::as_str) != Some(session)
        || record.get("agent").and_then(Value::as_str) != Some("claude")
        || record.pointer("/runtime/launch_id").and_then(Value::as_str) != Some(runtime)
    {
        return Err(unavailable());
    }
    let receipt: Value = serde_json::from_slice(receipt).map_err(|_| unavailable())?;
    let id = receipt
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && !id.chars().any(char::is_control)
                && !id.starts_with("local:v1:")
        })
        .ok_or_else(unavailable)?;
    let bytes = serde_json::to_vec(&json!({
        "schema_version":"agent-session.provider-conversation.v1",
        "runtime_id":runtime, "runtime_generation":generation,
        "provider":"claude", "session_id":id
    }))
    .map_err(|_| unavailable())?;
    nils_common::fs::write_atomic(&dir.join("provider-conversation.json"), &bytes, 0o600)
        .map_err(|_| unavailable())
}
