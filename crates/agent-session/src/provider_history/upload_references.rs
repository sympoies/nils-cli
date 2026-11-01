//! Quoted uploads are projected only after session ownership and raster checks.
//! The transcript and native model input are never rewritten.
use super::*;

impl HistoryCatalog {
    pub(crate) fn project_upload_references(
        &self,
        session: &HistorySession,
        messages: &mut [HistoryMessage],
        include_parts: bool,
    ) {
        if matches!(session.provider.as_str(), "codex" | "claude") {
            for message in messages.iter_mut().filter(|message| {
                !message.images.is_empty()
                    || (message.role == "user" && message.text.contains(['\'', '"']))
            }) {
                let Some(offset) = message
                    .id
                    .rsplit_once('-')
                    .and_then(|(_, n)| n.parse().ok())
                else {
                    continue;
                };
                let Ok(mut file) = fs::File::open(&session.transcript_path) else {
                    continue;
                };
                if file.seek(SeekFrom::Start(offset)).is_err() {
                    continue;
                }
                let mut line = Vec::new();
                if read_image_line(&mut BufReader::new(file), &mut line).ok()
                    != Some(BoundedLine::Line)
                {
                    continue;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if message.role == "user"
                    && message.text.contains(['\'', '"'])
                    && let Some((projected, images)) =
                        self.upload_projection(session, offset, &value)
                    && let Some((_, text, _, _)) = normalize_message(
                        &session.provider,
                        &session.provider_session_id,
                        &line,
                        &projected,
                    )
                {
                    message.text = truncate_chars(text.trim(), MAX_MESSAGE_CHARS);
                    message.images = images;
                    if include_parts {
                        message.parts = Some(
                            history_parts::record_parts(
                                &session.provider,
                                session.provider_version.as_deref(),
                                &projected,
                                true,
                                &message.images,
                                &message.id,
                            )
                            .0,
                        );
                    }
                }
                self.correlate_image_uploads(session, &value, message);
            }
        }
        if include_parts {
            correlate_tool_status(session, messages);
        }
    }

    fn upload_projection(
        &self,
        session: &HistorySession,
        offset: u64,
        value: &Value,
    ) -> Option<(Value, Vec<HistoryImage>)> {
        let blocks = upload_content(&session.provider, value)?;
        let original = descriptors(session, offset, value);
        let mut original_index = 0;
        let mut images = Vec::new();
        let mut content = Vec::new();
        let mut changed = false;
        for (block_index, block) in blocks.iter().enumerate() {
            if matches!(
                block.get("type").and_then(Value::as_str),
                Some("input_text" | "text")
            ) && let Some(text) = block.get("text").and_then(Value::as_str)
            {
                let mut consumed = 0;
                for (start, end, path) in quoted_paths(text) {
                    if images.len() >= MAX_IMAGES || self.upload_file(session, path).is_err() {
                        continue;
                    }
                    append_original(
                        serde_json::json!({"type":"input_text","text":&text[consumed..start]}),
                        &original,
                        &mut original_index,
                        &mut images,
                        &mut content,
                    );
                    if images.len() >= MAX_IMAGES {
                        consumed = start;
                        break;
                    }
                    content.push(serde_json::json!({"type":"input_image","path":path}));
                    images.push(HistoryImage {
                        id: format!(
                            "{}.{}.u{}-{}.{}",
                            segment_key(session),
                            offset,
                            block_index,
                            start,
                            row_key(value)
                        ),
                        origin: Some(image_provenance::ImageOrigin::UserAttachment),
                        attachment_ref: self.owned_attachment_ref(session, path),
                    });
                    consumed = end;
                    changed = true;
                }
                append_original(
                    serde_json::json!({"type":"input_text","text":&text[consumed..]}),
                    &original,
                    &mut original_index,
                    &mut images,
                    &mut content,
                );
            } else {
                append_original(
                    block.clone(),
                    &original,
                    &mut original_index,
                    &mut images,
                    &mut content,
                );
            }
        }
        if !changed {
            return None;
        }
        let mut projected = value.clone();
        if session.provider == "claude" {
            for block in &mut content {
                if block["type"] == "input_text" {
                    block["type"] = Value::String("text".into());
                }
                if block["type"] == "input_image" {
                    block["type"] = Value::String("image".into());
                }
            }
            if projected["type"] == "attachment" {
                projected["attachment"]["prompt"] = Value::Array(content);
            } else {
                projected["message"]["content"] = Value::Array(content);
            }
        } else {
            projected["payload"]["content"] = Value::Array(content);
        }
        Some((projected, images))
    }

    pub(super) fn read_upload_reference(
        &self,
        session: &HistorySession,
        value: &Value,
        selector: &str,
    ) -> Result<ImageBytes, ImageReadError> {
        let (block, start) = selector
            .split_once('-')
            .ok_or(ImageReadError::Unavailable)?;
        let block = block
            .parse::<usize>()
            .map_err(|_| ImageReadError::Unavailable)?;
        let start = start
            .parse::<usize>()
            .map_err(|_| ImageReadError::Unavailable)?;
        let blocks = upload_content(&session.provider, value).ok_or(ImageReadError::Deleted)?;
        let text = blocks
            .get(block)
            .filter(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("input_text" | "text")
                )
            })
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .ok_or(ImageReadError::Deleted)?;
        let path = quoted_paths(text)
            .find(|(position, _, _)| *position == start)
            .map(|(_, _, path)| path)
            .ok_or(ImageReadError::Deleted)?;
        let file = self.upload_file(session, path)?;
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
            filename: self.image_filename(session, Path::new(path)),
        })
    }

    pub(super) fn upload_file(
        &self,
        session: &HistorySession,
        name: &str,
    ) -> Result<fs::File, ImageReadError> {
        if !matches!(session.provider.as_str(), "codex" | "claude") {
            return Err(ImageReadError::Unavailable);
        }
        let path = Path::new(name);
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ImageReadError::Unavailable);
        }
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "gif", "webp"].contains(&extension.as_str()) {
            return Err(ImageReadError::Unavailable);
        }
        // Always use the managed attachment boundary, even when cwd is an ancestor.
        let (root, relative) = self.attachment_root(session, path)?;
        let mut file = open_beneath(&root, &relative)?;
        let metadata = file.metadata().map_err(image_io_error)?;
        if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
            return Err(ImageReadError::Unavailable);
        }
        let mut header = [0; 12];
        let size = file.read(&mut header).map_err(image_io_error)?;
        sniff_mime(&header[..size]).ok_or(ImageReadError::Unavailable)?;
        file.seek(SeekFrom::Start(0)).map_err(image_io_error)?;
        Ok(file)
    }
}

fn upload_content<'a>(provider: &str, value: &'a Value) -> Option<Cow<'a, [Value]>> {
    match provider {
        "codex"
            if value["type"] == "response_item"
                && value["payload"]["type"] == "message"
                && value["payload"]["role"] == "user" =>
        {
            Some(Cow::Borrowed(
                value.pointer("/payload/content")?.as_array()?,
            ))
        }
        // Claude Code 2.1.312 user strings and 2.1.316 human queued strings
        // can contain quoted uploads without a native structured image.
        "claude" => {
            let content = if value["type"] == "user" && value["message"]["role"] == "user" {
                value.pointer("/message/content")?
            } else {
                crate::provider_prompt::claude_queued_prompt_value(value)?
            };
            if let Some(text) = content.as_str() {
                Some(Cow::Owned(vec![
                    serde_json::json!({"type":"text","text":text}),
                ]))
            } else {
                Some(Cow::Borrowed(content.as_array()?))
            }
        }
        _ => None,
    }
}

fn quoted_paths(text: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        let start = text[offset..]
            .match_indices(['\'', '"'])
            .find_map(|(position, _)| {
                let start = offset + position;
                (text.as_bytes().get(start + 1) == Some(&b'/')).then_some(start)
            })?;
        let quote = text.as_bytes()[start] as char;
        let end = start + 1 + text[start + 1..].find(quote)?;
        offset = end + 1;
        Some((start, offset, &text[start + 1..end]))
    })
}

fn append_original(
    block: Value,
    original: &[HistoryImage],
    index: &mut usize,
    images: &mut Vec<HistoryImage>,
    content: &mut Vec<Value>,
) {
    let mut sources = Vec::new();
    let wrapper = Value::Array(vec![block.clone()]);
    collect_images(&wrapper, &mut sources, 0);
    let end = (*index + sources.len()).min(original.len());
    images.extend(
        original[*index..end]
            .iter()
            .take(MAX_IMAGES - images.len())
            .cloned(),
    );
    *index = end;
    content.push(block);
}
