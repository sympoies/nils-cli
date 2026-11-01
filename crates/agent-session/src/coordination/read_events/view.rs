//! The wire element for one event, as pinned by the fixture corpus.
use serde_json::{Value, json};

use super::store::Event;
use crate::CliContext;
use crate::notice::NoticeEnvelope;

const MARKER: &str = "<!-- agent-notice:";

/// Display-safe sender: no incarnation, generation or capability values.
pub(super) fn sender(raw: &Value, context: &CliContext) -> Value {
    let machine = |raw: &Value| {
        raw["machine"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| crate::board::machine_identity(None, context))
    };
    if raw["kind"] == "service" {
        json!({"kind": "service", "machine": machine(raw), "service_id": raw["service_id"]})
    } else {
        json!({"kind": "session", "machine": machine(raw), "session_id": raw["session_id"]})
    }
}

fn timestamp(ms: i64) -> Option<String> {
    let at = jiff::Timestamp::from_millisecond(ms).ok()?;
    Some(at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

pub(crate) fn message(history_id: &str, event: &Event, include_parts: bool) -> Option<Value> {
    let (body, body_prefix) = if event.body.starts_with(MARKER) {
        (format!("\n{}", event.body), "lf")
    } else {
        (event.body.clone(), "none")
    };
    let text = NoticeEnvelope::new("mailbox.read", "agent-session", Some(&event.message_id))
        .ok()?
        .encode(&body)
        .ok()?;
    let optional = |ms: Option<i64>| ms.and_then(timestamp);
    let mut message = json!({
        "id": format!("{history_id}-{}-mailbox", event.message_id),
        "role": "notice",
        "text": text,
        "timestamp": timestamp(event.created_at_ms)?,
        "mailbox": {
            "schema_version": "agent-session.mailbox-read-event.v1",
            "message_id": event.message_id,
            "sender": event.sender,
            "category": event.category,
            "state": if event.acknowledged_at_ms.is_some() { "acknowledged" } else { "read" },
            "read_at": optional(event.read_at_ms),
            "acknowledged_at": optional(event.acknowledged_at_ms),
            "revision": event.revision,
            "availability": "complete",
            "body_prefix": body_prefix,
        },
    });
    if include_parts {
        message["parts"] = json!([]);
    }
    Some(message)
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod view_tests;
