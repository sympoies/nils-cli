//! Recognition of current notice envelopes and provider context; it does not authorize delivery.
use serde_json::Value;

pub(crate) fn matches(provider: &str, text: &str, value: &Value) -> bool {
    if text.starts_with("<!-- agent-notice:") {
        return crate::notice::parse_notice(text).is_some();
    }
    provider == "codex"
        && value
            .pointer("/payload/internal_chat_message_metadata_passthrough/content_item_kinds")
            .is_none()
        && (frame(text, "<environment_context>", "</environment_context>")
            || instruction_frame(text))
}

fn frame(text: &str, open: &str, close: &str) -> bool {
    let Some(body) = text
        .strip_prefix(open)
        .and_then(|s| s.strip_prefix('\n'))
        .and_then(|s| s.strip_suffix('\n').or(Some(s)))
        .and_then(|s| s.strip_suffix(close))
        .and_then(|s| s.strip_suffix('\n'))
    else {
        return false;
    };
    !body.is_empty() && !body.contains(open) && !body.contains(close)
}
fn instruction_frame(text: &str) -> bool {
    let Some((title, body)) = text
        .strip_prefix("# AGENTS.md instructions for ")
        .and_then(|s| s.split_once("\n\n"))
    else {
        return false;
    };
    !title.is_empty() && !title.contains('\n') && frame(body, "<INSTRUCTIONS>", "</INSTRUCTIONS>")
}

pub(crate) fn input(provider: &str, value: &Value, session: &str) -> Option<String> {
    let text = match provider {
        "claude" if !super::claude_foreign(value, session) => {
            super::super::claude_user_message_text(value).or_else(|| {
                super::super::claude_queued_prompt_text(value).map(|s| s.into_owned())
            })?
        }
        "codex" if value["type"] == "event_msg" && value["payload"]["type"] == "user_message" => {
            value["payload"]["message"].as_str()?.to_owned()
        }
        "codex"
            if value["type"] == "response_item"
                && value["payload"]["type"] == "message"
                && value["payload"]["role"] == "user"
                && !value
                    .pointer(
                        "/payload/internal_chat_message_metadata_passthrough/content_item_kinds",
                    )
                    .and_then(Value::as_array)
                    .is_some_and(|k| {
                        !k.iter()
                            .any(|x| matches!(x.as_str(), Some("user.text" | "user.image")))
                    }) =>
        {
            value["payload"]["content"]
                .as_array()?
                .iter()
                .filter_map(|v| v["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    matches(provider, &text, value).then_some(text)
}

/// Select human previews without changing provider prompt admission.
pub(crate) fn human_history_prompt(
    provider: &str,
    session: &str,
    line: &str,
) -> Option<super::super::LastPrompt> {
    let prompt = super::super::parse_history_user_prompt(provider, session, line)?;
    (!matches("", &prompt.text, &Value::Null)).then_some(prompt)
}
