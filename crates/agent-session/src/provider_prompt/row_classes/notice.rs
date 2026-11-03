//! Recognition of known unmarked history inputs; it does not authorize delivery.
use serde_json::Value;

const EXACT_LINES: &[&str] = &[
    "Coordination mailbox has unread messages; run agent-session message inbox.",
    "壓縮完成；執行自己角色的 session-start，接著續做。",
];

pub(crate) fn matches(provider: &str, text: &str, value: &Value) -> bool {
    if text.starts_with("<!-- agent-notice:") {
        return crate::notice::parse_notice(text).is_some();
    }
    if let Some(rest) = text.strip_prefix("※ ")
        && let Some((title, body)) = rest.split_once('：')
        && !body.is_empty()
        && crate::notice::labels::NOTICE_LABELS
            .iter()
            .any(|entry| entry.label == title || entry.stored_title == Some(title))
    {
        return true;
    }
    if EXACT_LINES.contains(&text) || explained(text) {
        return true;
    }
    if let Some(date) = text
        .strip_prefix("Coordination mailbox has unread messages (newest queued ")
        .and_then(|s| s.strip_suffix("); run agent-session message inbox."))
        && canonical_date(date)
    {
        return true;
    }
    if let Some(date) = text
        .strip_prefix("Mailbox: ")
        .and_then(|s| s.split_once("; "))
        .map(|(date, _)| date)
        && date.as_bytes().get(4) == Some(&b'-')
        && !canonical_date(date)
    {
        return false;
    }
    if !text.starts_with(char::is_whitespace)
        && !text.starts_with("Coordination mailbox")
        && crate::coordination::is_mailbox_reminder_prompt(text)
    {
        return true;
    }
    provider == "codex"
        && value
            .pointer("/payload/internal_chat_message_metadata_passthrough/content_item_kinds")
            .is_none()
        && (frame(text, "<environment_context>", "</environment_context>")
            || instruction_frame(text))
}

fn explained(text: &str) -> bool {
    let Some(body) = text
        .strip_prefix("Mailbox: ")
        .and_then(|s| s.strip_suffix("; agent-session message inbox"))
    else {
        return false;
    };
    let categories = [
        "blocker",
        "decision",
        "handoff",
        "progress",
        "report",
        "uncategorized",
    ];
    let mut previous = None;
    for part in body.split(", ") {
        let Some((count, category)) = part.split_once(' ') else {
            return false;
        };
        let Some(index) = categories.iter().position(|c| *c == category) else {
            return false;
        };
        if previous.is_some_and(|p| p >= index)
            || !count
                .parse::<u16>()
                .is_ok_and(|n| (1..=256).contains(&n) && n.to_string() == count)
        {
            return false;
        }
        previous = Some(index);
    }
    previous.is_some()
}
fn canonical_date(date: &str) -> bool {
    date.len() == 20
        && date
            .parse::<jiff::Timestamp>()
            .is_ok_and(|v| v.to_string() == date)
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
