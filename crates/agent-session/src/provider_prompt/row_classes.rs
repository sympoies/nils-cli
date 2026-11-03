//! Display classification only; these tokens never authorize runtime input.
pub(crate) mod notice;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RowClass {
    RuntimeCommand,
    Compaction,
    Notice,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Compaction {
    pub(crate) summary_truncated: bool,
}

const CLAUDE_COMMANDS: &[&str] = &[
    "/compact", "/exit", "/quit", "/effort", "/model", "/usage", "/login", "/clear", "/new",
];
const CODEX_COMMANDS: &[&str] = &[
    "/compact", "/exit", "/quit", "/effort", "/model", "/usage", "/login", "/clear", "/new",
];
pub(crate) const HISTORY_TEXT_CHARS: usize = 65_536;
pub(crate) const ADJACENT_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn runtime_command(provider: &str, text: &str) -> bool {
    if text.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']) {
        return false;
    }
    let commands = match provider {
        "claude" => CLAUDE_COMMANDS,
        "codex" => CODEX_COMMANDS,
        _ => return false,
    };
    let token = text.split([' ', '\t']).next().unwrap_or_default();
    commands.contains(&token)
}

pub(crate) fn human_text(provider: &str, text: &str) -> bool {
    let projected = (provider == "claude")
        .then(|| super::claude_slash_command_text(text))
        .flatten();
    !runtime_command(provider, projected.as_deref().unwrap_or(text))
        && !notice::matches("", text, &Value::Null)
}

pub(crate) fn command_text(provider: &str, value: &Value) -> Option<String> {
    let text = match provider {
        "claude" => {
            let text = super::claude_user_message_text(value).or_else(|| {
                super::claude_queued_prompt_text(value).map(|text| text.into_owned())
            })?;
            // #3014: only the command-markup echo represents compact input.
            if runtime_command(provider, &text)
                && text.split([' ', '\t']).next() == Some("/compact")
            {
                return None;
            }
            super::claude_slash_command_text(&text).unwrap_or(text)
        }
        "codex" => {
            let payload = value.get("payload")?;
            match value.get("type")?.as_str()? {
                "event_msg" if payload.get("type")?.as_str()? == "user_message" => {
                    payload.get("message")?.as_str()?.to_owned()
                }
                "response_item"
                    if payload.get("type")?.as_str()? == "message"
                        && payload.get("role")?.as_str()? == "user" =>
                {
                    if payload
                        .pointer("/internal_chat_message_metadata_passthrough/content_item_kinds")
                        .and_then(Value::as_array)
                        .is_some_and(|kinds| {
                            !kinds.iter().any(|kind| {
                                matches!(kind.as_str(), Some("user.text" | "user.image"))
                            })
                        })
                    {
                        return None;
                    }
                    payload
                        .get("content")?
                        .as_array()?
                        .iter()
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    runtime_command(provider, &text).then_some(text)
}

pub(crate) fn claude_foreign(value: &Value, session: &str) -> bool {
    value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(Value::as_str)
        .is_some_and(|id| id != session)
}

pub(crate) fn claude_boundary(value: &Value) -> bool {
    value["type"] == "system"
        && value["subtype"] == "compact_boundary"
        && value["isSidechain"] != true
}

pub(crate) fn claude_summary(value: &Value) -> Option<String> {
    if value["type"] != "user" || value["isCompactSummary"] != true || value["isSidechain"] == true
    {
        return None;
    }
    let content = value.pointer("/message/content")?;
    Some(if let Some(text) = content.as_str() {
        text.to_owned()
    } else {
        content
            .as_array()?
            .iter()
            .filter(|item| item["type"] == "text")
            .filter_map(|item| item["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub(crate) fn claude_logical_record(value: &Value) -> bool {
    value["isSidechain"] != true
        && (matches!(
            value["type"].as_str(),
            Some("user" | "assistant" | "attachment")
        ) || claude_boundary(value))
}

pub(crate) fn bounded_text(text: &str) -> (String, bool) {
    let mut chars = text.chars();
    let text = chars.by_ref().take(HISTORY_TEXT_CHARS).collect();
    (text, chars.next().is_some())
}

pub(crate) fn summary(text: &str) -> (String, Compaction) {
    let (text, summary_truncated) = bounded_text(text);
    (text, Compaction { summary_truncated })
}
