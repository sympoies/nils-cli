use super::*;
use crate::provider_prompt::claude_queued_prompt_blocks;
use std::borrow::Cow;

#[path = "messages.rs"]
pub(crate) mod messages;

#[path = "history_parts.rs"]
mod history_parts;

#[cfg(test)]
#[path = "compact_tests.rs"]
mod compact_tests;

#[path = "upload_references.rs"]
mod upload_references;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct HistoryMessage {
    pub(crate) id: String,
    #[serde(skip)]
    pub(crate) history_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_request: Option<super::PromptRequestBinding>,
    pub(crate) role: String,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) images: Vec<HistoryImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parts: Option<Vec<history_parts::HistoryPart>>,
    #[serde(skip)]
    pub(crate) human_prompt: bool,
}

const MAX_TOOL_STATUS_CONTEXT_BYTES: u64 = 1024 * 1024;
const MAX_TOOL_STATUS_CONTEXT_LINES: usize = 1024;

pub(super) fn correlate_tool_status(session: &HistorySession, messages: &mut [HistoryMessage]) {
    let results = messages
        .iter()
        .filter_map(|message| message.id.rsplit_once('-')?.1.parse::<u64>().ok())
        .max()
        .map(|offset| {
            matching_tool_results(
                &session.transcript_path,
                &session.provider,
                session.provider_version.as_deref(),
                offset,
            )
        })
        .unwrap_or_default();
    history_parts::correlate_tool_status(messages, &results);
}

fn matching_tool_results(
    path: &Path,
    provider: &str,
    provider_version: Option<&str>,
    offset: u64,
) -> Vec<history_parts::HistoryPart> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return Vec::new();
    }
    let mut reader = BufReader::new(&mut file).take(MAX_TOOL_STATUS_CONTEXT_BYTES);
    let mut line = Vec::new();
    let mut results = Vec::new();
    for _ in 0..MAX_TOOL_STATUS_CONTEXT_LINES {
        match read_image_line(&mut reader, &mut line) {
            Ok(BoundedLine::Line) => {}
            Ok(BoundedLine::Eof | BoundedLine::Oversized) | Err(_) => break,
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let (parts, _) = history_parts::record_parts(
            provider,
            provider_version,
            &value,
            false,
            &[],
            "status-context",
        );
        results.extend(
            parts
                .into_iter()
                .filter(|part| matches!(part, history_parts::HistoryPart::ToolResult { .. })),
        );
    }
    results
}

pub(super) fn projected_timestamp(value: &Value) -> Option<String> {
    history_parts::projected_timestamp(value)
}

pub(super) fn provider_version(path: &Path, provider: &str) -> Option<String> {
    let mut reader = BufReader::new(fs::File::open(path).ok()?).take(64 * 1024);
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).ok()?;
    let value: Value = serde_json::from_slice(&line).ok()?;
    match provider {
        "codex" => value
            .pointer("/payload/cli_version")
            .or_else(|| value.get("cli_version"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

pub(super) fn read_messages(
    session: &HistorySession,
    offset: u64,
    limit: usize,
    include_parts: bool,
) -> Result<HistoryMessagesPage, HistoryError> {
    let file = fs::File::open(&session.transcript_path).map_err(|_| HistoryError::NotFound)?;
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| HistoryError::InvalidCursor)?;
    let mut messages = Vec::new();
    let mut line = Vec::new();
    let mut scanned = 0usize;
    let deadline = Instant::now() + MESSAGE_SCAN_MAX_DURATION;
    while messages.len() < limit {
        if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
            let next = reader.stream_position().map_err(|_| HistoryError::Io)?;
            return Ok(HistoryMessagesPage {
                messages,
                next_cursor: Some(next.to_string()),
                older_cursor: None,
            });
        }
        line.clear();
        let line_offset = reader.stream_position().map_err(|_| HistoryError::Io)?;
        let bounded = read_image_line(&mut reader, &mut line).map_err(|_| HistoryError::Io)?;
        if bounded == BoundedLine::Eof {
            return Ok(HistoryMessagesPage {
                messages,
                next_cursor: None,
                older_cursor: None,
            });
        }
        scanned += 1;
        if bounded == BoundedLine::Oversized {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if line.len() > MAX_LINE_BYTES && image_sources(&value).is_empty() {
            continue;
        }
        let normalized = normalize_message(
            &session.provider,
            &session.provider_session_id,
            &line,
            &value,
        );
        let images = descriptors(session, line_offset, &value);
        let (parts, part_role) = if include_parts {
            history_parts::record_parts(
                &session.provider,
                session.provider_version.as_deref(),
                &value,
                normalized.is_some(),
                &images,
                &format!("{}-{line_offset}", session.id),
            )
        } else {
            (Vec::new(), None)
        };
        let Some((role, text, timestamp, human_prompt)) = normalized.or_else(|| {
            (!parts.is_empty()).then(|| {
                (
                    part_role.unwrap_or("notice").to_string(),
                    String::new(),
                    projected_timestamp(&value),
                    false,
                )
            })
        }) else {
            continue;
        };
        if text.trim().is_empty() && images.is_empty() && parts.is_empty() {
            continue;
        }
        messages.push(HistoryMessage {
            id: format!("{}-{line_offset}", session.id),
            history_turn_id: projected_history_turn_id(&session.provider, &value, &role),
            prompt_request: None,
            role,
            text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
            timestamp,
            human_prompt,
            images,
            parts: include_parts.then_some(parts),
        });
    }
    let next = reader.stream_position().map_err(|_| HistoryError::Io)?;
    Ok(HistoryMessagesPage {
        messages,
        next_cursor: Some(next.to_string()),
        older_cursor: None,
    })
}

pub(super) fn read_compacted_image_line<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
    offset: u64,
    initial: &[u8],
) -> Result<ImageLineRead, HistoryError> {
    let mut raw = initial.to_vec();
    let mut position = offset.saturating_add(initial.len() as u64);
    reader
        .seek(SeekFrom::Start(position))
        .map_err(|_| HistoryError::Io)?;
    loop {
        let remaining = file_len
            .saturating_sub(position)
            .min((MAX_IMAGE_RECORD_BYTES + 1 - raw.len()) as u64);
        if remaining == 0 {
            return Ok(if raw.len() > MAX_IMAGE_RECORD_BYTES {
                ImageLineRead::Unsupported
            } else {
                ImageLineRead::Incomplete
            });
        }
        let mut chunk = vec![0; remaining.min(REVERSE_MESSAGE_CHUNK_BYTES as u64) as usize];
        reader
            .read_exact(&mut chunk)
            .map_err(|_| HistoryError::Io)?;
        position = position.saturating_add(chunk.len() as u64);
        if let Some(end) = chunk.iter().position(|byte| *byte == b'\n') {
            raw.extend_from_slice(&chunk[..end]);
            if raw.len() > MAX_IMAGE_RECORD_BYTES {
                return Ok(ImageLineRead::Unsupported);
            }
            let Ok(value) = serde_json::from_slice::<Value>(&raw) else {
                return Ok(ImageLineRead::Unsupported);
            };
            if image_sources(&value).is_empty() {
                return Ok(ImageLineRead::Unsupported);
            }
            let line = raw;
            return Ok(ImageLineRead::Complete(
                line,
                position - chunk.len() as u64 + end as u64 + 1,
            ));
        }
        raw.extend_from_slice(&chunk);
    }
}

pub(super) fn push_incremental_message(
    session: &HistorySession,
    line: &[u8],
    line_offset: u64,
    messages: &mut Vec<HistoryMessage>,
) {
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    if line.len() > MAX_LINE_BYTES && image_sources(&value).is_empty() {
        return;
    }
    let Some((role, text, timestamp, human_prompt)) = normalize_message(
        &session.provider,
        &session.provider_session_id,
        line,
        &value,
    ) else {
        return;
    };
    let images = descriptors(session, line_offset, &value);
    if text.trim().is_empty() && images.is_empty() {
        return;
    }
    let base_id = incremental_message_id(
        session,
        &value,
        &role,
        text.trim(),
        timestamp.as_deref(),
        human_prompt,
    );
    // Preserve IDs already stored in semantic memory for text-bearing rows.
    let id = if images.is_empty() || !text.trim().is_empty() {
        base_id
    } else {
        digest_parts(&[base_id.as_bytes(), row_key(&value).as_bytes()])
    };
    messages.push(HistoryMessage {
        id,
        history_turn_id: projected_history_turn_id(&session.provider, &value, &role),
        prompt_request: None,
        role,
        text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
        timestamp,
        human_prompt,
        images,
        parts: None,
    });
}

pub(super) fn push_reverse_message(
    session: &HistorySession,
    raw_line: &[u8],
    line_offset: u64,
    messages: &mut Vec<HistoryMessage>,
    include_parts: bool,
) {
    let raw_line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
    if raw_line.is_empty() || raw_line.len() > MAX_IMAGE_RECORD_BYTES {
        return;
    }
    let line = raw_line;
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    if line.len() > MAX_LINE_BYTES && image_sources(&value).is_empty() {
        return;
    }
    let normalized = normalize_message(
        &session.provider,
        &session.provider_session_id,
        line,
        &value,
    );
    let images = descriptors(session, line_offset, &value);
    let (parts, part_role) = if include_parts {
        history_parts::record_parts(
            &session.provider,
            session.provider_version.as_deref(),
            &value,
            normalized.is_some(),
            &images,
            &format!("{}-{line_offset}", session.id),
        )
    } else {
        (Vec::new(), None)
    };
    let Some((role, text, timestamp, human_prompt)) = normalized.or_else(|| {
        (!parts.is_empty()).then(|| {
            (
                part_role.unwrap_or("notice").to_string(),
                String::new(),
                projected_timestamp(&value),
                false,
            )
        })
    }) else {
        return;
    };
    if text.trim().is_empty() && images.is_empty() && parts.is_empty() {
        return;
    }
    messages.push(HistoryMessage {
        id: format!("{}-{line_offset}", session.id),
        history_turn_id: projected_history_turn_id(&session.provider, &value, &role),
        prompt_request: None,
        role,
        text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
        timestamp,
        human_prompt,
        images,
        parts: include_parts.then_some(parts),
    });
}

fn projected_history_turn_id(provider: &str, value: &Value, role: &str) -> Option<String> {
    if provider != "codex" || role != "user" {
        return None;
    }
    let kinds = value
        .pointer("/payload/internal_chat_message_metadata_passthrough/content_item_kinds")?
        .as_array()?;
    if !kinds
        .iter()
        .any(|kind| matches!(kind.as_str(), Some("user.text" | "user.image")))
    {
        return None;
    }
    value
        .pointer("/payload/internal_chat_message_metadata_passthrough/turn_id")
        .and_then(Value::as_str)
        .filter(|id| {
            !id.trim().is_empty()
                && id.len() <= 256
                && !id.contains(['/', '\\'])
                && !id.contains("://")
        })
        .map(str::to_string)
}

pub(super) fn normalize_message(
    provider: &str,
    provider_session_id: &str,
    raw_line: &[u8],
    value: &Value,
) -> Option<(String, String, Option<String>, bool)> {
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    match provider {
        "codex" => {
            if value.get("type").and_then(Value::as_str) == Some("event_msg") {
                let payload = value.get("payload")?;
                if payload.get("type").and_then(Value::as_str) != Some("thread_goal_updated")
                    || payload.get("threadId").and_then(Value::as_str) != Some(provider_session_id)
                {
                    return None;
                }
                let goal = payload.get("goal")?;
                if goal.get("threadId").and_then(Value::as_str) != Some(provider_session_id)
                    || goal.get("status").and_then(Value::as_str) != Some("active")
                {
                    return None;
                }
                let created_at = goal.get("createdAt")?.as_u64()?;
                let event_second = timestamp
                    .as_deref()?
                    .parse::<jiff::Timestamp>()
                    .ok()
                    .and_then(|stamp| u64::try_from(stamp.as_second()).ok())?;
                if event_second != created_at {
                    return None;
                }
                let objective = goal.get("objective")?.as_str()?.trim();
                if objective.is_empty() || objective.len() > MAX_MESSAGE_CHARS {
                    return None;
                }
                return Some(("goal".to_string(), objective.to_string(), timestamp, true));
            }
            if value.get("type").and_then(Value::as_str) != Some("response_item") {
                return None;
            }
            let payload = value.get("payload")?;
            if matches!(
                payload.get("type").and_then(Value::as_str),
                Some("function_call_output" | "custom_tool_call_output" | "mcp_tool_result")
            ) && !image_sources(value).is_empty()
            {
                return Some(("assistant".to_string(), String::new(), timestamp, false));
            }
            if payload.get("type").and_then(Value::as_str) != Some("message") {
                return None;
            }
            let role = payload.get("role")?.as_str()?;
            if !matches!(role, "user" | "assistant") {
                return None;
            }
            let human_prompt = role == "user"
                && parse_history_user_prompt(
                    provider,
                    provider_session_id,
                    std::str::from_utf8(raw_line).ok()?,
                )
                .is_some();
            // Older Codex transcripts have no content_item_kinds. Keep those
            // rows visible, but do not promote them into retitle memory: an
            // unclassified user frame could also be injected context.
            let classified_non_human = payload
                .pointer("/internal_chat_message_metadata_passthrough/content_item_kinds")
                .and_then(Value::as_array)
                .is_some_and(|kinds| {
                    !kinds
                        .iter()
                        .any(|kind| matches!(kind.as_str(), Some("user.text" | "user.image")))
                });
            if role == "user" && !human_prompt && classified_non_human {
                return None;
            }
            let text = content_text(payload.get("content")?);
            Some((role.to_string(), text, timestamp, human_prompt))
        }
        "claude" => {
            // A prompt sent while a turn was running is an attachment row.
            let role = match value.get("type")?.as_str()? {
                "attachment" => "user",
                role @ ("user" | "assistant") => role,
                _ => return None,
            };
            if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let text = if role == "user" {
                claude_user_message_text(value)
                    .or_else(|| claude_queued_prompt_text(value).map(Cow::into_owned))
                    .or_else(|| {
                        (!image_sources(value).is_empty()
                            && value.get("isMeta") != Some(&Value::Bool(true)))
                        .then(String::new)
                    })?
            } else {
                content_text(value.get("message")?.get("content")?)
            };
            // Keep the command-markup echo for each compaction. Its literal
            // input can carry arguments and promptSource, and is separated
            // from the echo by the compact boundary and summary.
            if role == "user" && text.split_whitespace().next() == Some("/compact") {
                return None;
            }
            let human_prompt = role == "user"
                && parse_history_user_prompt(
                    provider,
                    provider_session_id,
                    std::str::from_utf8(raw_line).ok()?,
                )
                .is_some();
            if role == "user" && !human_prompt && image_sources(value).is_empty() {
                return None;
            }
            let text = if role == "user" {
                claude_slash_command_text(&text).unwrap_or(text)
            } else {
                text
            };
            // Claude stores tool output in user envelopes. Keep real prompts
            // on the user side, but attribute image-only tool output to the agent.
            let role = if role == "user"
                && !human_prompt
                && value
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        !blocks.is_empty()
                            && blocks.iter().all(|block| {
                                block.get("type").and_then(Value::as_str) == Some("tool_result")
                            })
                    }) {
                "assistant"
            } else {
                role
            };
            Some((role.to_string(), text, timestamp, human_prompt))
        }
        _ => None,
    }
}

fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .or_else(|| part.get("content").and_then(Value::as_str))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// References are source-bound selectors, never URLs or client-supplied paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct HistoryImage {
    pub(crate) id: String,
}

const MAX_IMAGES: usize = 16;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
enum ImageSource<'a> {
    Url(&'a str),
    Encoded(&'a str, &'a str),
    File(&'a str),
    Unsupported,
}

fn image_sources(value: &Value) -> Vec<ImageSource<'_>> {
    let mut sources = Vec::new();
    let content = value
        .pointer("/payload/content")
        .or_else(|| {
            matches!(
                value.pointer("/payload/type").and_then(Value::as_str),
                Some("function_call_output" | "custom_tool_call_output" | "mcp_tool_result")
            )
            .then(|| value.pointer("/payload/output"))
            .flatten()
        })
        .or_else(|| value.pointer("/message/content"))
        .or_else(|| claude_queued_prompt_blocks(value));
    if let Some(content) = content {
        collect_images(content, &mut sources, 0);
    }
    sources
}

fn collect_images<'a>(content: &'a Value, sources: &mut Vec<ImageSource<'a>>, depth: usize) {
    if depth > 2 || sources.len() >= MAX_IMAGES {
        return;
    }
    if let Some(text) = content.as_str() {
        collect_markdown_images(text, sources);
        return;
    }
    for part in content.as_array().into_iter().flatten() {
        if sources.len() >= MAX_IMAGES {
            break;
        }
        match part.get("type").and_then(Value::as_str).unwrap_or_default() {
            "input_image" | "output_image" | "image" => {
                let source = if let Some(url) = part
                    .get("image_url")
                    .and_then(Value::as_str)
                    .or_else(|| part.pointer("/image_url/url").and_then(Value::as_str))
                    .or_else(|| part.pointer("/source/url").and_then(Value::as_str))
                {
                    ImageSource::Url(url)
                } else if let Some(path) = part
                    .get("path")
                    .and_then(Value::as_str)
                    .or_else(|| part.pointer("/source/path").and_then(Value::as_str))
                {
                    ImageSource::File(path)
                } else if part.pointer("/source/type").and_then(Value::as_str) == Some("base64") {
                    ImageSource::Encoded(
                        part.pointer("/source/media_type")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        part.pointer("/source/data")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )
                } else {
                    ImageSource::Unsupported
                };
                sources.push(source);
            }
            "text" | "input_text" | "output_text" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    collect_markdown_images(text, sources);
                }
            }
            "tool_result" => {
                if let Some(content) = part.get("content") {
                    collect_images(content, sources, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn collect_markdown_images<'a>(mut text: &'a str, sources: &mut Vec<ImageSource<'a>>) {
    while sources.len() < MAX_IMAGES {
        let Some((_, rest)) = text.split_once("![") else {
            break;
        };
        let Some((_, rest)) = rest.split_once("](") else {
            break;
        };
        let Some((target, rest)) = rest.split_once(')') else {
            break;
        };
        let target = target.trim();
        let target = target
            .strip_prefix('<')
            .and_then(|v| v.split_once('>').map(|v| v.0))
            .unwrap_or_else(|| target.split_whitespace().next().unwrap_or_default());
        sources.push(if target.starts_with("data:") || target.contains("://") {
            ImageSource::Url(target)
        } else {
            ImageSource::File(target)
        });
        text = rest;
    }
}

fn segment_key(session: &HistorySession) -> String {
    use std::os::unix::ffi::OsStrExt;
    let hash = digest_parts(&[
        session.id.as_bytes(),
        session.transcript_path.as_os_str().as_bytes(),
    ]);
    hash[7..23].to_string()
}

fn row_key(value: &Value) -> String {
    let hash = digest_parts(&[&serde_json::to_vec(value).unwrap_or_default()]);
    hash[7..39].to_string()
}

fn descriptors(session: &HistorySession, offset: u64, value: &Value) -> Vec<HistoryImage> {
    let sources = image_sources(value);
    if sources.is_empty() {
        return Vec::new();
    }
    let segment = segment_key(session);
    let row = row_key(value);
    sources
        .iter()
        .enumerate()
        .map(|(index, _)| HistoryImage {
            id: format!("{segment}.{offset}.{index}.{row}"),
        })
        .collect()
}

fn read_image_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
) -> std::io::Result<BoundedLine> {
    output.clear();
    let read = reader
        .take(MAX_IMAGE_RECORD_BYTES as u64 + 2)
        .read_until(b'\n', output)?;
    if read == 0 {
        return Ok(BoundedLine::Eof);
    }
    if output.len() > MAX_IMAGE_RECORD_BYTES + 1
        || (output.len() == MAX_IMAGE_RECORD_BYTES + 1 && output.last() != Some(&b'\n'))
    {
        if output.last() != Some(&b'\n') {
            let mut discard = Vec::new();
            loop {
                discard.clear();
                let read = reader.take(64 * 1024).read_until(b'\n', &mut discard)?;
                if read == 0 || discard.last() == Some(&b'\n') {
                    break;
                }
            }
        }
        return Ok(BoundedLine::Oversized);
    }
    Ok(BoundedLine::Line)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ImageReadError {
    Deleted,
    Unavailable,
    OutsideAllowedRoots,
}
pub(crate) struct ImageBytes {
    pub(crate) mime: &'static str,
    pub(crate) bytes: Vec<u8>,
    pub(crate) filename: Option<String>,
}

impl HistoryCatalog {
    pub(crate) fn image(
        &self,
        history_id: &str,
        image_id: &str,
    ) -> Result<ImageBytes, ImageReadError> {
        let parts: Vec<_> = image_id.split('.').collect();
        if parts.len() != 4
            || parts[0].len() != 16
            || parts[3].len() != 32
            || !parts[0]
                .bytes()
                .chain(parts[3].bytes())
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(ImageReadError::Unavailable);
        }
        let offset = parts[1]
            .parse::<u64>()
            .map_err(|_| ImageReadError::Unavailable)?;
        let index = if parts[2].starts_with('u') {
            None
        } else {
            Some(
                parts[2]
                    .parse::<usize>()
                    .ok()
                    .filter(|index| *index < MAX_IMAGES)
                    .ok_or(ImageReadError::Unavailable)?,
            )
        };
        let snapshot = self.snapshot();
        let sessions = snapshot
            .incremental_segments
            .get(history_id)
            .ok_or(ImageReadError::Deleted)?;
        let session = sessions
            .iter()
            .find(|s| segment_key(s) == parts[0])
            .ok_or(ImageReadError::Deleted)?;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&session.transcript_path)
            .map_err(image_io_error)?;
        let metadata = file.metadata().map_err(image_io_error)?;
        if !metadata.is_file() || offset >= metadata.len() {
            return Err(ImageReadError::Deleted);
        }
        if offset > 0 {
            file.seek(SeekFrom::Start(offset - 1))
                .map_err(image_io_error)?;
            let mut boundary = [0];
            file.read_exact(&mut boundary).map_err(image_io_error)?;
            if boundary != *b"\n" {
                return Err(ImageReadError::Deleted);
            }
        }
        file.seek(SeekFrom::Start(offset)).map_err(image_io_error)?;
        let mut line = Vec::new();
        if read_image_line(&mut BufReader::new(file), &mut line).map_err(image_io_error)?
            != BoundedLine::Line
        {
            return Err(ImageReadError::Unavailable);
        }
        let value: Value = serde_json::from_slice(&line).map_err(|_| ImageReadError::Deleted)?;
        if row_key(&value) != parts[3]
            || normalize_message(
                &session.provider,
                &session.provider_session_id,
                &line,
                &value,
            )
            .is_none()
        {
            return Err(ImageReadError::Deleted);
        }
        let Some(index) = index else {
            return self.read_upload_reference(session, &value, &parts[2][1..]);
        };
        let sources = image_sources(&value);
        match sources.get(index).ok_or(ImageReadError::Deleted)? {
            ImageSource::Url(url) => {
                let Some((mime, encoded)) = url
                    .strip_prefix("data:")
                    .and_then(|v| v.split_once(";base64,"))
                else {
                    return Err(ImageReadError::Unavailable);
                };
                decode_image(mime, encoded)
            }
            ImageSource::Encoded(mime, encoded) => decode_image(mime, encoded),
            ImageSource::File(path) => self.file_image(session, path),
            ImageSource::Unsupported => Err(ImageReadError::Unavailable),
        }
    }

    fn file_image(
        &self,
        session: &HistorySession,
        name: &str,
    ) -> Result<ImageBytes, ImageReadError> {
        let name = name.strip_prefix("file://").unwrap_or(name);
        let candidate = Path::new(name);
        if candidate
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ImageReadError::Unavailable);
        }
        let cwd = Path::new(&session.cwd);
        let path = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        let (root, relative) = if let Ok(relative) = path.strip_prefix(cwd) {
            (cwd.to_path_buf(), relative.to_path_buf())
        } else {
            self.attachment_root(session, &path)
                .map_err(|error| match error {
                    ImageReadError::Unavailable => ImageReadError::OutsideAllowedRoots,
                    error => error,
                })?
        };
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "gif", "webp"].contains(&extension.as_str()) {
            return Err(ImageReadError::Unavailable);
        }
        let file = open_beneath(&root, &relative)?;
        let metadata = file.metadata().map_err(image_io_error)?;
        if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
            return Err(ImageReadError::Unavailable);
        }
        let mut bytes = Vec::new();
        file.take(MAX_IMAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(image_io_error)?;
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            return Err(ImageReadError::Unavailable);
        }
        let mime = sniff_mime(&bytes).ok_or(ImageReadError::Unavailable)?;
        Ok(ImageBytes {
            mime,
            bytes,
            filename: self.image_filename(session, &path),
        })
    }

    fn image_filename(&self, session: &HistorySession, path: &Path) -> Option<String> {
        if let Ok((root, relative)) = self.attachment_root(session, path) {
            let metadata = crate::attachment_filename::metadata_path(&relative)?;
            let mut file = open_beneath(&root, &metadata).ok()?;
            let info = file.metadata().ok()?;
            if !info.is_file() || info.len() > 4096 {
                return None;
            }
            let mut bytes = Vec::new();
            file.by_ref().take(4097).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 4096 {
                return None;
            }
            let name: String = serde_json::from_slice(&bytes).ok()?;
            return crate::attachment_filename::sanitize_download_filename(&name);
        }
        crate::attachment_filename::sanitize_download_filename(path.file_name()?.to_str()?)
    }

    fn attachment_root(
        &self,
        session: &HistorySession,
        path: &Path,
    ) -> Result<(PathBuf, PathBuf), ImageReadError> {
        let state = self
            .archives_root
            .parent()
            .and_then(Path::parent)
            .ok_or(ImageReadError::Unavailable)?;
        let relative = path
            .strip_prefix(state)
            .map_err(|_| ImageReadError::Unavailable)?;
        let parts: Vec<_> = relative.components().collect();
        if parts.len() != 4
            || parts[0].as_os_str() != "sessions"
            || parts[2].as_os_str() != "attachments"
        {
            return Err(ImageReadError::Unavailable);
        }
        let id = parts[1]
            .as_os_str()
            .to_str()
            .ok_or(ImageReadError::Unavailable)?;
        let archived = read_archive(&self.archives_root, &session.id)
            .and_then(|a| a.session_id)
            .is_some_and(|s| s == id);
        let context = crate::CliContext::resolve(Some(state.to_path_buf()), None)
            .map_err(|_| ImageReadError::Unavailable)?;
        let live = crate::load_session_record(&context, id)
            .ok()
            .is_some_and(|r| {
                r.provider_resume.is_some_and(|p| {
                    p.provider == session.provider && p.session_id == session.provider_session_id
                })
            });
        if !archived && !live {
            return Err(ImageReadError::Deleted);
        }
        Ok((state.to_path_buf(), relative.to_path_buf()))
    }
}

fn image_io_error(error: std::io::Error) -> ImageReadError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ImageReadError::Deleted
    } else {
        ImageReadError::Unavailable
    }
}

fn open_beneath(root: &Path, relative: &Path) -> Result<fs::File, ImageReadError> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    let root = fs::canonicalize(root).map_err(image_io_error)?;
    let mut directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(root)
        .map_err(image_io_error)?;
    let components: Vec<_> = relative
        .components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .collect();
    if components.is_empty() {
        return Err(ImageReadError::Unavailable);
    }
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return Err(ImageReadError::Unavailable);
        };
        let name =
            std::ffi::CString::new(name.as_bytes()).map_err(|_| ImageReadError::Unavailable)?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC
            | if index + 1 < components.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(image_io_error(std::io::Error::last_os_error()));
        }
        directory = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(directory)
}

fn decode_image(mime: &str, encoded: &str) -> Result<ImageBytes, ImageReadError> {
    use base64::Engine;
    if !["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&mime)
        || encoded.len() as u64 > MAX_IMAGE_BYTES.div_ceil(3) * 4
    {
        return Err(ImageReadError::Unavailable);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ImageReadError::Unavailable)?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(ImageReadError::Unavailable);
    }
    let actual = sniff_mime(&bytes).ok_or(ImageReadError::Unavailable)?;
    if actual != mime {
        return Err(ImageReadError::Unavailable);
    }
    Ok(ImageBytes {
        mime: actual,
        bytes,
        filename: None,
    })
}

fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

pub(super) fn read_messages_reverse(
    session: &HistorySession,
    before: Option<u64>,
    limit: usize,
    include_parts: bool,
) -> Result<HistoryMessagesPage, HistoryError> {
    let deadline = Instant::now() + MESSAGE_SCAN_MAX_DURATION;
    let mut file = fs::File::open(&session.transcript_path).map_err(|_| HistoryError::NotFound)?;
    let file_len = file.metadata().map_err(|_| HistoryError::Io)?.len();
    read_messages_reverse_from_reader(
        session,
        &mut file,
        file_len,
        before,
        limit,
        deadline,
        include_parts,
    )
}

pub(super) fn read_messages_reverse_from_reader<R: Read + Seek>(
    session: &HistorySession,
    reader: &mut R,
    file_len: u64,
    before: Option<u64>,
    limit: usize,
    deadline: Instant,
    include_parts: bool,
) -> Result<HistoryMessagesPage, HistoryError> {
    let end = before.unwrap_or(file_len);
    if end > file_len {
        return Err(HistoryError::InvalidCursor);
    }
    if end == 0 {
        return Ok(HistoryMessagesPage {
            messages: Vec::new(),
            next_cursor: None,
            older_cursor: None,
        });
    }
    if before.is_some() {
        reader
            .seek(SeekFrom::Start(end.saturating_sub(1)))
            .map_err(|_| HistoryError::InvalidCursor)?;
        let mut preceding = [0u8; 1];
        reader
            .read_exact(&mut preceding)
            .map_err(|_| HistoryError::InvalidCursor)?;
        if preceding[0] != b'\n' {
            return Err(HistoryError::InvalidCursor);
        }
    }

    let lower_bound = end.saturating_sub(REVERSE_MESSAGE_MAX_BYTES);
    let mut position = end;
    let mut partial: Vec<Vec<u8>> = Vec::new();
    let mut partial_len = 0usize;
    let mut partial_oversized = false;
    let mut messages = Vec::new();
    let mut bound_reached = false;
    let mut oldest_offset = None;
    let mut earliest_boundary = None;
    let mut scanned = 0usize;

    'chunks: while position > lower_bound && messages.len() < limit {
        if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
            bound_reached = true;
            break;
        }
        let chunk_start = position
            .saturating_sub(REVERSE_MESSAGE_CHUNK_BYTES as u64)
            .max(lower_bound);
        let chunk_len = usize::try_from(position - chunk_start).map_err(|_| HistoryError::Io)?;
        let mut chunk = vec![0u8; chunk_len];
        reader
            .seek(SeekFrom::Start(chunk_start))
            .map_err(|_| HistoryError::Io)?;
        reader
            .read_exact(&mut chunk)
            .map_err(|_| HistoryError::Io)?;
        position = chunk_start;

        let mut right = chunk.len();
        let mut rightmost = true;
        while let Some(newline) = chunk[..right].iter().rposition(|byte| *byte == b'\n') {
            if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
                bound_reached = true;
                break 'chunks;
            }
            let line_start = newline + 1;
            let line_offset = chunk_start.saturating_add(line_start as u64);
            earliest_boundary = Some(line_offset);
            scanned += 1;

            if rightmost && (partial_len > 0 || partial_oversized) {
                if !partial_oversized
                    && chunk.len().saturating_sub(line_start) + partial_len
                        <= MAX_IMAGE_RECORD_BYTES
                {
                    let mut joined =
                        Vec::with_capacity(chunk.len().saturating_sub(line_start) + partial_len);
                    joined.extend_from_slice(&chunk[line_start..]);
                    for fragment in partial.iter().rev() {
                        joined.extend_from_slice(fragment);
                    }
                    push_reverse_message(
                        session,
                        &joined,
                        line_offset,
                        &mut messages,
                        include_parts,
                    );
                }
                partial.clear();
                partial_len = 0;
                partial_oversized = false;
            } else {
                push_reverse_message(
                    session,
                    &chunk[line_start..right],
                    line_offset,
                    &mut messages,
                    include_parts,
                );
            }
            if messages.len() == limit {
                break 'chunks;
            }
            right = newline;
            rightmost = false;
        }

        if chunk_start == 0 {
            if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
                bound_reached = true;
                break;
            }
            earliest_boundary = Some(0);
            if rightmost && (partial_len > 0 || partial_oversized) {
                if !partial_oversized && chunk.len() + partial_len <= MAX_IMAGE_RECORD_BYTES {
                    let mut joined = Vec::with_capacity(chunk.len() + partial_len);
                    joined.extend_from_slice(&chunk);
                    for fragment in partial.iter().rev() {
                        joined.extend_from_slice(fragment);
                    }
                    push_reverse_message(session, &joined, 0, &mut messages, include_parts);
                }
            } else {
                push_reverse_message(session, &chunk[..right], 0, &mut messages, include_parts);
            }
            break;
        }

        let fragment = &chunk[..right];
        if rightmost {
            if partial_oversized || fragment.len() + partial_len > MAX_IMAGE_RECORD_BYTES {
                partial.clear();
                partial_len = 0;
                partial_oversized = true;
            } else {
                partial.push(fragment.to_vec());
                partial_len += fragment.len();
            }
        } else if fragment.len() > MAX_IMAGE_RECORD_BYTES {
            partial.clear();
            partial_len = 0;
            partial_oversized = true;
        } else {
            partial.clear();
            partial.push(fragment.to_vec());
            partial_len = fragment.len();
        }
    }

    for message in &messages {
        let Some(offset) = message
            .id
            .rsplit('-')
            .next()
            .and_then(|value| value.parse().ok())
        else {
            continue;
        };
        oldest_offset = Some(oldest_offset.map_or(offset, |current: u64| current.min(offset)));
    }
    messages.reverse();
    let unread_older = position > 0
        || lower_bound > 0
        || bound_reached
        || (messages.len() == limit && oldest_offset.is_some_and(|offset| offset > 0));
    let continuation =
        oldest_offset.or_else(|| unread_older.then_some(earliest_boundary).flatten());
    let older_cursor = continuation
        .filter(|offset| *offset > 0 && unread_older)
        .map(|offset| offset.to_string());
    Ok(HistoryMessagesPage {
        messages,
        next_cursor: None,
        older_cursor,
    })
}

#[cfg(test)]
mod queued_list_prompt_tests;

#[cfg(test)]
mod image_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parts_query_serializes_empty_array_while_legacy_shape_omits_field() {
        let message = HistoryMessage {
            id: "message".to_string(),
            history_turn_id: None,
            prompt_request: None,
            role: "user".to_string(),
            text: "text".to_string(),
            timestamp: None,
            images: Vec::new(),
            parts: Some(Vec::new()),
            human_prompt: true,
        };
        let typed = serde_json::to_value(&message).unwrap();
        assert_eq!(typed["parts"], serde_json::json!([]));

        let mut default_shape = message;
        default_shape.parts = None;
        assert!(
            serde_json::to_value(default_shape)
                .unwrap()
                .get("parts")
                .is_none()
        );
    }

    #[test]
    fn history_image_data_and_descriptor_bounds_are_explicit() {
        assert!(decode_image("image/svg+xml", "PHN2Zz4=").is_err());
        assert!(decode_image("image/png", "invalid base64!").is_err());
        assert!(decode_image("image/jpeg", "iVBORw0KGgo=").is_err());
        assert!(
            decode_image(
                "image/png",
                &"A".repeat((MAX_IMAGE_BYTES.div_ceil(3) * 4 + 1) as usize)
            )
            .is_err()
        );
        assert_eq!(
            decode_image("image/png", "iVBORw0KGgo=").unwrap().mime,
            "image/png"
        );
        let content: Vec<_> = (0..100).map(|_| serde_json::json!({"type":"input_image","image_url":"https://example.invalid/image.png"})).collect();
        let value = serde_json::json!({"payload":{"content":content}});
        assert_eq!(image_sources(&value).len(), MAX_IMAGES);
    }

    #[test]
    fn history_image_file_walk_refuses_parent_symlinks_and_non_regular_files() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("image.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
        assert!(open_beneath(&root, Path::new("linked/image.png")).is_err());
        assert!(open_beneath(&root, Path::new("../outside/image.png")).is_err());
        assert!(open_beneath(&root, Path::new("missing.png")).is_err());
    }
}
