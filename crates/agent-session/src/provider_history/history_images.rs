use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct HistoryMessage {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) images: Vec<HistoryImage>,
    #[serde(skip)]
    pub(crate) human_prompt: bool,
}

pub(super) fn read_messages(
    session: &HistorySession,
    offset: u64,
    limit: usize,
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
        let Some((role, text, timestamp, human_prompt)) = normalize_message(
            &session.provider,
            &session.provider_session_id,
            &line,
            &value,
        ) else {
            continue;
        };
        let images = descriptors(session, line_offset, &value);
        if text.trim().is_empty() && images.is_empty() {
            continue;
        }
        messages.push(HistoryMessage {
            id: format!("{}-{line_offset}", session.id),
            role,
            text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
            timestamp,
            human_prompt,
            images,
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
        role,
        text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
        timestamp,
        human_prompt,
        images,
    });
}

pub(super) fn push_reverse_message(
    session: &HistorySession,
    raw_line: &[u8],
    line_offset: u64,
    messages: &mut Vec<HistoryMessage>,
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
    messages.push(HistoryMessage {
        id: format!("{}-{line_offset}", session.id),
        role,
        text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
        timestamp,
        human_prompt,
        images,
    });
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
                    .or_else(|| claude_queued_prompt_text(value).map(str::to_string))
                    .or_else(|| {
                        (!image_sources(value).is_empty()
                            && value.get("isMeta") != Some(&Value::Bool(true)))
                        .then(String::new)
                    })?
            } else {
                content_text(value.get("message")?.get("content")?)
            };
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
        .or_else(|| value.pointer("/message/content"));
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
}
pub(crate) struct ImageBytes {
    pub(crate) mime: &'static str,
    pub(crate) bytes: Vec<u8>,
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
        let index = parts[2]
            .parse::<usize>()
            .map_err(|_| ImageReadError::Unavailable)?;
        if index >= MAX_IMAGES {
            return Err(ImageReadError::Unavailable);
        }
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
            self.attachment_root(session, &path)?
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
        Ok(ImageBytes { mime, bytes })
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

#[cfg(test)]
mod image_tests {
    use super::*;
    use pretty_assertions::assert_eq;

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
