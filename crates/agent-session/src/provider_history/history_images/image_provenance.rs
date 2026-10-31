//! Per-image provider provenance; correlation still requires managed ownership.
use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImageOrigin {
    UserAttachment,
    ToolResult,
}

pub(super) fn origins(provider: &str, value: &Value) -> Vec<Option<ImageOrigin>> {
    use ImageOrigin::{ToolResult, UserAttachment};
    // Codex 0.x.y response items and Claude Code 2.1.x message/tool blocks.
    // An assistant envelope alone never proves that an image is tool output.
    let origin = match provider {
        "codex" if value["type"] == "response_item" => match value["payload"]["type"].as_str() {
            Some("message") => match value["payload"]["role"].as_str() {
                Some("user") => Some(UserAttachment),
                Some("assistant") => None,
                _ => return Vec::new(),
            },
            Some("function_call_output" | "custom_tool_call_output" | "mcp_tool_result") => {
                Some(ToolResult)
            }
            _ => return Vec::new(),
        },
        "claude" if value["type"] == "user" && value["message"]["role"] == "user" => {
            // Claude can carry nonhuman context in a user envelope. Keep its
            // image visibility, but only nested tool blocks prove an origin.
            (value["promptSource"] != "system"
                && value["isCompactSummary"] != true
                && value["isVisibleInTranscriptOnly"] != true)
                .then_some(UserAttachment)
        }
        "claude" if value["type"] == "assistant" && value["message"]["role"] == "assistant" => None,
        "claude" if claude_queued_prompt_blocks(value).is_some() => Some(UserAttachment),
        _ => return Vec::new(),
    };
    let mut output = Vec::new();
    if let Some(content) = image_content(value) {
        collect_origins(content, origin, &mut output, 0);
    }
    output
}

fn collect_origins(
    content: &Value,
    origin: Option<ImageOrigin>,
    output: &mut Vec<Option<ImageOrigin>>,
    depth: usize,
) {
    if depth > 2 || output.len() >= MAX_IMAGES {
        return;
    }
    if let Some(text) = content.as_str() {
        let mut sources = Vec::new();
        collect_markdown_images(text, &mut sources);
        output.extend(std::iter::repeat_n(
            origin,
            sources.len().min(MAX_IMAGES - output.len()),
        ));
        return;
    }
    for block in content.as_array().into_iter().flatten() {
        if output.len() >= MAX_IMAGES {
            break;
        }
        match block["type"].as_str() {
            Some("image" | "input_image" | "output_image") => output.push(origin),
            Some("text" | "input_text" | "output_text") => {
                if let Some(text) = block.get("text").filter(|v| v.is_string()) {
                    collect_origins(text, origin, output, depth);
                }
            }
            Some("tool_result") => {
                if let Some(content) = block.get("content") {
                    collect_origins(content, Some(ImageOrigin::ToolResult), output, depth + 1);
                }
            }
            _ => {}
        }
    }
}

impl HistoryCatalog {
    pub(super) fn owned_attachment_ref(
        &self,
        session: &HistorySession,
        name: &str,
    ) -> Option<String> {
        use std::os::unix::fs::MetadataExt;
        let name = name.strip_prefix("file://").unwrap_or(name);
        // Ownership, no-follow opens, size, extension and raster checks run
        // before minting metadata, even if cwd contains the upload directory.
        let file = self.upload_file(session, name).ok()?;
        let metadata = file.metadata().ok()?;
        // Hash opened file identity, never paths, content or display names.
        // Equivalent supported paths correlate; newly allocated uploads do not,
        // even when an old upload path is reused.
        let hash = digest_parts(&[
            session.id.as_bytes(),
            &metadata.dev().to_le_bytes(),
            &metadata.ino().to_le_bytes(),
            &metadata.ctime().to_le_bytes(),
            &metadata.ctime_nsec().to_le_bytes(),
        ]);
        Some(format!("upload-{}", &hash[7..]))
    }

    pub(super) fn correlate_image_uploads(
        &self,
        session: &HistorySession,
        value: &Value,
        message: &mut HistoryMessage,
    ) {
        let sources = image_sources(value);
        for image in &mut message.images {
            // Select by the original row index: quoted projections can insert
            // descriptors between structured/Markdown sources.
            let source = image
                .id
                .split('.')
                .nth(2)
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| sources.get(index));
            if let Some(ImageSource::File(path)) = source {
                let candidate = Path::new(path.strip_prefix("file://").unwrap_or(path));
                let path = if candidate.is_absolute() {
                    candidate.to_path_buf()
                } else {
                    Path::new(&session.cwd).join(candidate)
                };
                image.attachment_ref = path
                    .to_str()
                    .and_then(|path| self.owned_attachment_ref(session, path));
            }
        }
        for part in message.parts.iter_mut().flatten() {
            if let history_parts::HistoryPart::Image {
                image_ref,
                origin,
                attachment_ref,
                ..
            } = part
                && let Some(image) = message.images.iter().find(|image| &image.id == image_ref)
            {
                *origin = image.origin;
                *attachment_ref = image.attachment_ref.clone();
            }
        }
        // Correlation adds bytes after the initial parts projection.
        message.parts = message.parts.take().map(history_parts::bound_parts);
    }
}
