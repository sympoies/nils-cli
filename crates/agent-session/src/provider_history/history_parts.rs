use super::MAX_MESSAGE_CHARS;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

const MAX_PART_SUMMARY_CHARS: usize = 2_048;
const MAX_QUESTION_CHARS: usize = 4_096;
const MAX_PARTS_PER_MESSAGE: usize = 256;
const MAX_PART_BYTES_PER_MESSAGE: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum HistoryPart {
    Text {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        text: String,
    },
    ToolCall {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        name: String,
        input_summary: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        started_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ended_at: Option<String>,
    },
    ToolResult {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        summary: String,
        truncated: bool,
        is_error: bool,
    },
    Image {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[serde(rename = "ref")]
        image_ref: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        bytes: Option<u64>,
    },
    Question {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        source: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Notice {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        notice: String,
    },
}

pub(super) fn correlate_tool_status(
    messages: &mut [super::HistoryMessage],
    extra_results: &[HistoryPart],
) {
    let statuses: HashMap<_, _> = messages
        .iter()
        .flat_map(|message| message.parts.iter().flatten())
        .filter_map(|part| match part {
            HistoryPart::ToolResult {
                call_id: Some(call_id),
                is_error,
                timestamp,
                ..
            } => Some((call_id.clone(), (*is_error, timestamp.clone()))),
            _ => None,
        })
        .chain(extra_results.iter().filter_map(|part| match part {
            HistoryPart::ToolResult {
                call_id: Some(call_id),
                is_error,
                timestamp,
                ..
            } => Some((call_id.clone(), (*is_error, timestamp.clone()))),
            _ => None,
        }))
        .collect();
    for part in messages
        .iter_mut()
        .flat_map(|message| message.parts.iter_mut().flatten())
    {
        if let HistoryPart::ToolCall {
            call_id: Some(call_id),
            status,
            ended_at,
            ..
        } = part
            && let Some((is_error, timestamp)) = statuses.get(call_id)
        {
            *status = if *is_error { "failed" } else { "completed" }.to_string();
            *ended_at = timestamp.clone();
        }
    }
}

pub(super) fn projected_timestamp(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .filter(|timestamp| timestamp.len() <= 128 && !appears_path_or_url(timestamp))
        .map(str::to_string)
}

pub(crate) fn record_parts(
    provider: &str,
    provider_version: Option<&str>,
    value: &Value,
    visible_message: bool,
    images: &[super::HistoryImage],
    fallback_id: &str,
) -> (Vec<HistoryPart>, Option<&'static str>) {
    let Some((id, timestamp)) = provider_item_identity(provider, value, fallback_id) else {
        return (Vec::new(), None);
    };
    let mut parts = Vec::new();
    let mut role = None;
    match provider {
        "codex" if supported_codex_version(provider_version) => codex_parts(
            value,
            visible_message,
            &id,
            timestamp.as_deref(),
            images,
            &mut parts,
            &mut role,
        ),
        "claude" => claude_parts(
            value,
            visible_message,
            &id,
            timestamp.as_deref(),
            images,
            &mut parts,
            &mut role,
        ),
        _ => {}
    }
    (bound_parts(parts), role)
}

fn bound_parts(parts: Vec<HistoryPart>) -> Vec<HistoryPart> {
    let mut output = Vec::new();
    let mut used: usize = 2;
    for mut part in parts {
        let encoded_len = serde_json::to_vec(&part).map_or(usize::MAX, |bytes| bytes.len());
        let separator = usize::from(!output.is_empty());
        if used.saturating_add(separator).saturating_add(encoded_len) <= MAX_PART_BYTES_PER_MESSAGE
        {
            used += separator + encoded_len;
            output.push(part);
            continue;
        }
        let remaining = MAX_PART_BYTES_PER_MESSAGE
            .saturating_sub(used)
            .saturating_sub(separator);
        if remaining <= 256 {
            break;
        }
        let max_chars = (remaining - 256) / 6;
        match &mut part {
            HistoryPart::Text { text, .. } => *text = truncate(text, max_chars),
            HistoryPart::ToolCall { input_summary, .. } => {
                *input_summary = truncate(input_summary, max_chars)
            }
            HistoryPart::ToolResult {
                summary, truncated, ..
            } => {
                *summary = truncate(summary, max_chars);
                *truncated = true;
            }
            HistoryPart::Question { text, .. } => {
                if let Some(text) = text {
                    *text = truncate(text, max_chars);
                }
            }
            HistoryPart::Image { .. } | HistoryPart::Notice { .. } => {}
        }
        let encoded_len = serde_json::to_vec(&part).map_or(usize::MAX, |bytes| bytes.len());
        if used.saturating_add(separator).saturating_add(encoded_len) <= MAX_PART_BYTES_PER_MESSAGE
        {
            output.push(part);
        }
        break;
    }
    output
}

fn supported_codex_version(version: Option<&str>) -> bool {
    let Some(version) = version else { return false };
    let mut components = version.split('.');
    matches!(
        (components.next(), components.next(), components.next(), components.next()),
        (Some("0"), Some(minor), Some(patch), None)
            if minor.parse::<u16>().is_ok() && patch.parse::<u16>().is_ok()
    )
}

fn provider_item_identity(
    provider: &str,
    value: &Value,
    fallback_id: &str,
) -> Option<(String, Option<String>)> {
    let timestamp = projected_timestamp(value);
    let item_id = match provider {
        "codex" => value.pointer("/payload/id").and_then(Value::as_str),
        "claude" => value.get("uuid").and_then(Value::as_str),
        _ => None,
    };
    if item_id.is_some_and(appears_path_or_url) {
        return None;
    }
    Some((bounded_id(item_id.unwrap_or(fallback_id)), timestamp))
}

fn bounded_id(value: &str) -> String {
    let value = value.trim();
    let value = if value.is_empty() { "unknown" } else { value };
    value.chars().take(256).collect()
}

fn tool_id(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| value.len() <= 256 && !appears_path_or_url(value))
        .map(bounded_id)
}

fn codex_parts(
    value: &Value,
    visible_message: bool,
    id: &str,
    timestamp: Option<&str>,
    images: &[super::HistoryImage],
    parts: &mut Vec<HistoryPart>,
    role: &mut Option<&'static str>,
) {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        if value.get("type").and_then(Value::as_str) == Some("event_msg") {
            let event = value.pointer("/payload/type").and_then(Value::as_str);
            if event == Some("thread_goal_updated") && visible_message {
                if let Some(objective) = value
                    .pointer("/payload/goal/objective")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    *role = Some("goal");
                    push_text(objective, id, timestamp, parts);
                }
            } else if let Some(notice) = codex_notice(event) {
                parts.push(HistoryPart::Notice {
                    id: id.to_string(),
                    timestamp: timestamp.map(str::to_string),
                    notice: notice.to_string(),
                });
            }
        }
        return;
    }
    let payload = &value["payload"];
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {
            let Some(message_role) = payload.get("role").and_then(Value::as_str) else {
                return;
            };
            *role = match message_role {
                "user" => Some("user"),
                "assistant" => Some("assistant"),
                _ => return,
            };
            if visible_message {
                push_visible_content(payload.get("content"), images, value, id, timestamp, parts);
            }
        }
        Some("function_call" | "custom_tool_call" | "mcp_tool_call" | "web_search_call") => {
            *role = Some("assistant");
            push_tool_call(
                payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool"),
                payload.get("arguments").or_else(|| payload.get("input")),
                id,
                tool_id(payload.get("call_id").and_then(Value::as_str)),
                timestamp,
                parts,
            );
        }
        Some("function_call_output" | "custom_tool_call_output" | "mcp_tool_result") => {
            *role = Some("assistant");
            push_tool_result(
                payload.get("output").or_else(|| payload.get("content")),
                payload
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                id,
                tool_id(payload.get("call_id").and_then(Value::as_str)),
                timestamp,
                parts,
            );
            push_images(images, value, id, timestamp, parts);
        }
        _ => {}
    }
}

fn claude_parts(
    value: &Value,
    visible_message: bool,
    id: &str,
    timestamp: Option<&str>,
    images: &[super::HistoryImage],
    parts: &mut Vec<HistoryPart>,
    role: &mut Option<&'static str>,
) {
    if !supported_claude_transcript_version(value.get("version").and_then(Value::as_str)) {
        return;
    }
    if value.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
        || value.get("isCompactSummary").and_then(Value::as_bool) == Some(true)
        || value
            .get("isVisibleInTranscriptOnly")
            .and_then(Value::as_bool)
            == Some(true)
        || value.get("promptSource").and_then(Value::as_str) == Some("system")
    {
        return;
    }
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    if matches!(kind, "user" | "assistant" | "attachment") {
        *role = Some(if matches!(kind, "user" | "attachment") {
            "user"
        } else {
            "assistant"
        });
        if kind == "attachment" {
            if visible_message {
                if let Some(text) = crate::provider_prompt::claude_queued_prompt_text(value) {
                    push_text(&text, id, timestamp, parts);
                }
                push_images(images, value, id, timestamp, parts);
            }
            return;
        }
        let Some(content) = value
            .pointer("/message/content")
            .or_else(|| value.get("content"))
        else {
            if visible_message {
                push_images(images, value, id, timestamp, parts);
            }
            return;
        };
        if visible_message {
            push_visible_content(Some(content), images, value, id, timestamp, parts);
        } else if let Some(blocks) = content.as_array() {
            for block in blocks {
                if parts.len() >= MAX_PARTS_PER_MESSAGE {
                    break;
                }
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") => push_tool_call(
                        block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                        block.get("input"),
                        id,
                        tool_id(block.get("id").and_then(Value::as_str)),
                        timestamp,
                        parts,
                    ),
                    Some("tool_result") => push_tool_result(
                        block.get("content"),
                        block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        id,
                        block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .and_then(|value| tool_id(Some(value))),
                        timestamp,
                        parts,
                    ),
                    _ => {}
                }
            }
        }
    } else if kind == "system"
        && let Some(notice) = claude_notice(value)
    {
        parts.push(HistoryPart::Notice {
            id: id.to_string(),
            timestamp: timestamp.map(str::to_string),
            notice: notice.to_string(),
        });
    }
}

fn supported_claude_transcript_version(version: Option<&str>) -> bool {
    let Some(version) = version else { return false };
    let mut components = version.split('.');
    matches!(
        (components.next(), components.next(), components.next(), components.next()),
        (Some("2"), Some("1"), Some(patch), None) if patch.parse::<u16>().is_ok()
    )
}

fn push_text(text: &str, id: &str, timestamp: Option<&str>, parts: &mut Vec<HistoryPart>) {
    if parts.len() >= MAX_PARTS_PER_MESSAGE {
        return;
    }
    let text = truncate(text.trim(), MAX_MESSAGE_CHARS);
    if !text.is_empty() {
        parts.push(HistoryPart::Text {
            id: id.to_string(),
            timestamp: timestamp.map(str::to_string),
            text,
        });
    }
}

fn push_tool_call(
    name: &str,
    input: Option<&Value>,
    id: &str,
    call_id: Option<String>,
    timestamp: Option<&str>,
    parts: &mut Vec<HistoryPart>,
) {
    if parts.len() >= MAX_PARTS_PER_MESSAGE {
        return;
    }
    let name = bounded_name(name);
    let source = question_source(&name);
    let parsed_input = input.map(parse_json_string).unwrap_or(Value::Null);
    let summary = bounded_summary(&parsed_input);
    parts.push(HistoryPart::ToolCall {
        id: id.to_string(),
        call_id: call_id.clone(),
        timestamp: timestamp.map(str::to_string),
        name: name.clone(),
        input_summary: summary,
        status: "unknown".to_string(),
        started_at: timestamp
            .filter(|_| !timestamp.is_some_and(appears_path_or_url))
            .map(str::to_string),
        ended_at: None,
    });
    if let Some(source) = source.filter(|_| parts.len() < MAX_PARTS_PER_MESSAGE) {
        let text = input
            .map(parse_json_string)
            .as_ref()
            .and_then(question_text)
            .map(|value| truncate(&value, MAX_QUESTION_CHARS));
        parts.push(HistoryPart::Question {
            id: id.to_string(),
            timestamp: timestamp.map(str::to_string),
            source: source.to_string(),
            text,
        });
    }
}

fn push_tool_result(
    output: Option<&Value>,
    is_error: bool,
    id: &str,
    call_id: Option<String>,
    timestamp: Option<&str>,
    parts: &mut Vec<HistoryPart>,
) {
    if parts.len() >= MAX_PARTS_PER_MESSAGE {
        return;
    }
    let mut safe_output = output.cloned().unwrap_or(Value::Null);
    redact_value(&mut safe_output, None);
    let raw = match &safe_output {
        Value::String(value) => value.clone(),
        value => serde_json::to_string(value).unwrap_or_default(),
    };
    let redacted = redact_text(&raw);
    let truncated = redacted.chars().count() > MAX_PART_SUMMARY_CHARS;
    parts.push(HistoryPart::ToolResult {
        id: id.to_string(),
        call_id,
        timestamp: timestamp.map(str::to_string),
        summary: truncate(&redacted, MAX_PART_SUMMARY_CHARS),
        truncated,
        is_error,
    });
}

fn push_images(
    images: &[super::HistoryImage],
    value: &Value,
    id: &str,
    timestamp: Option<&str>,
    parts: &mut Vec<HistoryPart>,
) {
    let sources = super::image_sources(value);
    push_image_range(images, &sources, 0, images.len(), id, timestamp, parts);
}

fn push_visible_content(
    content: Option<&Value>,
    images: &[super::HistoryImage],
    value: &Value,
    id: &str,
    timestamp: Option<&str>,
    parts: &mut Vec<HistoryPart>,
) {
    let Some(content) = content else {
        push_images(images, value, id, timestamp, parts);
        return;
    };
    let sources = super::image_sources(value);
    let mut image_index = 0;
    if let Some(blocks) = content.as_array() {
        for block in blocks {
            if parts.len() >= MAX_PARTS_PER_MESSAGE {
                break;
            }
            match block.get("type").and_then(Value::as_str) {
                Some("text" | "input_text" | "output_text") => {
                    let text = block
                        .get("text")
                        .or_else(|| block.get("content"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    push_text(text, id, timestamp, parts);
                    let count = markdown_image_count(text);
                    push_image_range(images, &sources, image_index, count, id, timestamp, parts);
                    image_index += count;
                }
                Some("input_image" | "output_image" | "image") => {
                    push_image_range(images, &sources, image_index, 1, id, timestamp, parts);
                    image_index += 1;
                }
                Some("tool_use") => push_tool_call(
                    block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                    block.get("input"),
                    id,
                    tool_id(block.get("id").and_then(Value::as_str)),
                    timestamp,
                    parts,
                ),
                Some("tool_result") => {
                    push_tool_result(
                        block.get("content"),
                        block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        id,
                        block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .and_then(|value| tool_id(Some(value))),
                        timestamp,
                        parts,
                    );
                    let mut nested = Vec::new();
                    if let Some(content) = block.get("content") {
                        super::collect_images(content, &mut nested, 0);
                    }
                    push_image_range(
                        images,
                        &sources,
                        image_index,
                        nested.len(),
                        id,
                        timestamp,
                        parts,
                    );
                    image_index += nested.len();
                }
                _ => {}
            }
        }
    } else if let Some(text) = content.as_str() {
        push_text(text, id, timestamp, parts);
        let count = markdown_image_count(text);
        push_image_range(images, &sources, image_index, count, id, timestamp, parts);
        image_index += count;
    }
    if image_index < images.len() {
        push_image_range(
            images,
            &sources,
            image_index,
            images.len() - image_index,
            id,
            timestamp,
            parts,
        );
    }
}

fn markdown_image_count(text: &str) -> usize {
    let mut sources = Vec::new();
    super::collect_markdown_images(text, &mut sources);
    sources.len()
}

fn push_image_range(
    images: &[super::HistoryImage],
    sources: &[super::ImageSource<'_>],
    start: usize,
    count: usize,
    id: &str,
    timestamp: Option<&str>,
    parts: &mut Vec<HistoryPart>,
) {
    for (index, image) in images
        .iter()
        .enumerate()
        .take(start.saturating_add(count).min(images.len()))
        .skip(start)
    {
        if parts.len() >= MAX_PARTS_PER_MESSAGE {
            break;
        }
        let (mime, bytes) = sources
            .get(index)
            .map(image_metadata)
            .unwrap_or((None, None));
        parts.push(HistoryPart::Image {
            id: id.to_string(),
            timestamp: timestamp.map(str::to_string),
            image_ref: image.id.clone(),
            mime,
            bytes,
        });
    }
}

fn image_metadata(source: &super::ImageSource<'_>) -> (Option<String>, Option<u64>) {
    match source {
        super::ImageSource::Encoded(mime, data) => (safe_mime(mime), base64_decoded_len(data)),
        super::ImageSource::Url(url) if url.starts_with("data:") => {
            let Some((header, data)) = url.split_once(',') else {
                return (None, None);
            };
            let mime = header
                .strip_prefix("data:")
                .and_then(|value| value.split(';').next())
                .and_then(safe_mime);
            (mime, base64_decoded_len(data))
        }
        _ => (None, None),
    }
}

fn safe_mime(value: &str) -> Option<String> {
    matches!(
        value,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
    .then(|| value.to_string())
}

fn base64_decoded_len(value: &str) -> Option<u64> {
    if !value.len().is_multiple_of(4)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return None;
    }
    let padding = value.bytes().rev().take_while(|byte| *byte == b'=').count();
    if padding > 2 {
        return None;
    }
    u64::try_from(value.len() / 4)
        .ok()?
        .checked_mul(3)?
        .checked_sub(padding as u64)
}

fn question_source(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "askuserquestion" | "ask_user_question" => Some("ask_user_question"),
        "request_user_input" | "requestuserinput" => Some("request_user_input"),
        "elicitation" | "elicitation/create" => Some("elicitation"),
        _ => None,
    }
}

fn question_text(input: &Value) -> Option<String> {
    let text = input
        .get("question")
        .and_then(Value::as_str)
        .or_else(|| input.get("prompt").and_then(Value::as_str))
        .or_else(|| {
            input
                .get("questions")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .and_then(|question| question.get("question"))
                .and_then(Value::as_str)
        })?;
    Some(redact_text(text))
}

fn bounded_summary(input: &Value) -> String {
    let mut value = input.clone();
    redact_value(&mut value, None);
    let summary = serde_json::to_string(&value).unwrap_or_default();
    truncate(&summary, MAX_PART_SUMMARY_CHARS)
}

fn parse_json_string(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| value.clone())
}

fn redact_value(value: &mut Value, key: Option<&str>) {
    if key.is_some_and(sensitive_key) {
        *value = Value::String("[redacted]".to_string());
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                redact_value(value, Some(key));
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_value(value, None);
            }
        }
        Value::String(text) => *text = redact_text(text),
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let normalized: String = key.chars().filter(char::is_ascii_alphanumeric).collect();
    [
        "path",
        "file",
        "directory",
        "cwd",
        "token",
        "secret",
        "password",
        "authorization",
        "cookie",
        "credential",
        "privatekey",
        "apikey",
        "accesskey",
        "clientsecret",
    ]
    .iter()
    .any(|needle| normalized.contains(needle) || key.contains(needle))
}

fn appears_path_or_url(value: &str) -> bool {
    let value = value.trim();
    let value = value.trim_matches(|character: char| {
        matches!(
            character,
            '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"' | ',' | ';' | ':'
        )
    });
    let value = value
        .rsplit_once('=')
        .or_else(|| {
            value
                .rsplit_once(':')
                .filter(|(prefix, _)| !prefix.contains('/'))
        })
        .map(|(_, value)| value.trim())
        .unwrap_or(value);
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("file:")
        || value.contains("://")
        || value.contains("\\")
        || looks_like_relative_path(value)
}

fn looks_like_relative_path(value: &str) -> bool {
    let value = value.trim_matches(|character: char| {
        matches!(
            character,
            '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"' | ',' | ';' | ':'
        )
    });
    let segments: Vec<_> = value.split('/').collect();
    segments.len() > 1
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "._-~".contains(character)
                })
        })
}

fn redact_text(value: &str) -> String {
    if appears_path_or_url(value) {
        "[redacted]".to_string()
    } else {
        let mut output = String::with_capacity(value.len());
        for segment in value.split_inclusive(char::is_whitespace) {
            let whitespace = segment.find(char::is_whitespace).unwrap_or(segment.len());
            let (word, spacing) = segment.split_at(whitespace);
            output.push_str(if appears_path_or_url(word) {
                "[redacted]"
            } else {
                word
            });
            output.push_str(spacing);
        }
        output
    }
}

fn bounded_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect()
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn codex_notice(event: Option<&str>) -> Option<&'static str> {
    match event? {
        "turn_aborted" => Some("interrupt"),
        "context_compacted" | "compaction_started" | "compaction_completed" => Some("compaction"),
        "auto_continue" => Some("auto_continuation"),
        _ => None,
    }
}

fn claude_notice(value: &Value) -> Option<&'static str> {
    match value.get("subtype").and_then(Value::as_str)? {
        "compact_boundary" => Some("compaction"),
        "interrupt" => Some("interrupt"),
        "auto_continue" => Some("auto_continuation"),
        _ => None,
    }
}

#[cfg(test)]
#[path = "history_parts_tests.rs"]
mod tests;
