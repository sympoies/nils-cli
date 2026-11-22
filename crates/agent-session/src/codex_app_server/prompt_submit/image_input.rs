//! Bounded native image input from the sending session's own upload directory.
use std::{
    io::Read,
    path::{Component, Path},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    CliContext, CliError,
    provider_history::history_images::{
        MAX_IMAGE_BYTES, MAX_IMAGES, open_beneath, quoted_paths, sniff_mime,
    },
};

const TOO_LARGE: &str = "prompt-images-too-large";
const TOO_MANY: &str = "prompt-too-many-images";
const UNSUPPORTED_TYPE: &str = "prompt-image-type-unsupported";
const UNREADABLE: &str = "prompt-image-unreadable";

pub(crate) fn refusal_message(code: &str) -> Option<&'static str> {
    match code {
        TOO_LARGE => Some("Codex prompt images exceed 8 MiB total"),
        TOO_MANY => Some("a Codex prompt can contain at most 16 images"),
        UNSUPPORTED_TYPE => Some(
            "Codex prompt images require matching PNG, JPEG, GIF, or WebP file types and signatures",
        ),
        UNREADABLE => Some("the session's uploaded image could not be read"),
        _ => None,
    }
}

fn refusal(code: &'static str) -> CliError {
    CliError::data(
        code,
        refusal_message(code).expect("known prompt image refusal"),
        Some(json!({"stage":"admission", "pasted":false, "submitted":false})),
    )
}

pub(crate) fn prepare(
    context: &CliContext,
    session_id: &str,
    text: &str,
) -> Result<Option<Vec<Value>>, CliError> {
    let relative_root = Path::new("sessions").join(session_id).join("attachments");
    let upload_dir = context.state_dir.join(&relative_root);
    let mut remaining = MAX_IMAGE_BYTES;
    let mut images = 0;
    let mut consumed = 0;
    let mut input = Vec::new();
    for (start, end, name) in quoted_paths(text) {
        let path = Path::new(name);
        let Ok(relative) = path.strip_prefix(&upload_dir) else {
            continue;
        };
        let mut components = relative.components();
        let Some(Component::Normal(leaf)) = components.next() else {
            continue;
        };
        if components.next().is_some() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        let mime = match extension.to_ascii_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" | "svgz" | "bmp" | "ico" | "tif" | "tiff" | "avif" | "heic" | "heif" | "jxl" => {
                return Err(refusal(UNSUPPORTED_TYPE));
            }
            _ => continue,
        };
        if images == MAX_IMAGES {
            return Err(refusal(TOO_MANY));
        }
        let file = open_beneath(&context.state_dir, &relative_root.join(leaf))
            .map_err(|_| refusal(UNREADABLE))?;
        let metadata = file.metadata().map_err(|_| refusal(UNREADABLE))?;
        if !metadata.is_file() {
            return Err(refusal(UNREADABLE));
        }
        if metadata.len() > remaining {
            return Err(refusal(TOO_LARGE));
        }
        let mut bytes = Vec::new();
        file.take(remaining + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| refusal(UNREADABLE))?;
        if bytes.len() as u64 > remaining {
            return Err(refusal(TOO_LARGE));
        }
        if sniff_mime(&bytes) != Some(mime) {
            return Err(refusal(UNSUPPORTED_TYPE));
        }
        push_text(&mut input, &text[consumed..start]);
        // Codex 0.162.1 and 0.171.0 TurnStartParams use UserInput::Image with a URL.
        let url = format!("data:{mime};base64,{}", STANDARD.encode(&bytes));
        input.push(json!({"type":"image", "url":url}));
        remaining -= bytes.len() as u64;
        images += 1;
        consumed = end;
    }
    if images == 0 {
        return Ok(None);
    }
    push_text(&mut input, &text[consumed..]);
    Ok(Some(input))
}

fn push_text(input: &mut Vec<Value>, text: &str) {
    if !text.is_empty() {
        input.push(json!({"type":"text", "text":text, "text_elements":[]}));
    }
}
