//! Content-free tool-call evidence shared by raw and dispatched Claude hooks.
//!
//! Claude's `PermissionRequest` payload names the tool and carries the same
//! `tool_input` as the `PreToolUse` that preceded it, but has no `tool_use_id`.
//! The evidence here lets the activity reducer bind the request to the call it
//! belongs to and clear it on that call's own result: the call id and an input
//! digest are runtime-scoped projections, and the tool input itself never
//! leaves this function.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::adapter::projected_provider_id;
use crate::model::Product;

const MAX_ID_CHARS: usize = 256;
const MAX_TOOL_NAME_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallPhase {
    /// `PreToolUse`: the call exists and has an id and an input digest.
    Started,
    /// `PermissionRequest`: a dialog is shown for a call with this input digest.
    Permission,
    /// `PostToolUse`, `PostToolUseFailure` or `PermissionDenied`: the call ended.
    Finished,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolCallEvidence {
    pub phase: ToolCallPhase,
    /// Runtime-scoped projection of `tool_use_id`; absent on `Permission`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Runtime-scoped projection of the tool name and input; absent on `Finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
    /// Bounded provider tool name, safe to show to clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ToolCallEvidence {
    /// Whether every field is within the bounds `evidence` produces.
    pub fn is_well_formed(&self) -> bool {
        let projected = |value: &Option<String>| {
            value.as_deref().is_none_or(|value| {
                value.strip_prefix("local:v1:").is_some_and(|digest| {
                    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
        };
        let required = match self.phase {
            ToolCallPhase::Started => self.call_id.is_some() && self.input_digest.is_some(),
            ToolCallPhase::Permission => {
                self.call_id.is_none() && (self.input_digest.is_some() || self.tool_name.is_some())
            }
            ToolCallPhase::Finished => self.call_id.is_some() && self.input_digest.is_none(),
        };
        required
            && projected(&self.call_id)
            && projected(&self.input_digest)
            && self.tool_name.as_deref().is_none_or(safe_tool_name)
    }
}

fn safe_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_TOOL_NAME_CHARS
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '-'))
}

/// Tool-call evidence for a Claude hook payload, or `None` when the event does
/// not carry enough to correlate. It never fails: optional metadata that is
/// missing or malformed leaves the approval on the conservative latch.
/// `AskUserQuestion` is excluded because it is already exact through its own
/// `tool_use_id`.
pub fn evidence(
    runtime_id: &str,
    event: &str,
    fields: &Map<String, Value>,
) -> Option<ToolCallEvidence> {
    let name = fields.get("tool_name").and_then(Value::as_str);
    if name == Some("AskUserQuestion") {
        return None;
    }
    let tool_name = name.filter(|name| safe_tool_name(name)).map(str::to_string);
    let call_id = || {
        let id = fields.get("tool_use_id").and_then(Value::as_str)?;
        let valid = !id.is_empty()
            && id.chars().count() <= MAX_ID_CHARS
            && !id.chars().any(char::is_control);
        valid.then(|| projected_provider_id(runtime_id, Product::Claude, "tool-call", id))
    };
    let input_digest = || {
        let input = serde_json::to_string(fields.get("tool_input")?).ok()?;
        let identity = format!("{}\0{input}", name?);
        Some(projected_provider_id(
            runtime_id,
            Product::Claude,
            "tool-input",
            &identity,
        ))
    };
    let (phase, call_id, input_digest) = match event {
        "PreToolUse" => (
            ToolCallPhase::Started,
            Some(call_id()?),
            Some(input_digest()?),
        ),
        "PermissionRequest" => (ToolCallPhase::Permission, None, input_digest()),
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
            (ToolCallPhase::Finished, Some(call_id()?), None)
        }
        _ => return None,
    };
    if phase == ToolCallPhase::Permission && input_digest.is_none() && tool_name.is_none() {
        return None;
    }
    Some(ToolCallEvidence {
        phase,
        call_id,
        input_digest,
        tool_name,
    })
}
