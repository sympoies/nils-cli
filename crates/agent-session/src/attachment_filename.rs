//! Original display names are separate from sanitized on-disk upload names.
use crate::{CliError, ensure_private_dir};
use axum::http::HeaderValue;
use jiff::Zoned;
use std::path::{Path, PathBuf};

pub(crate) fn sanitize_download_filename(raw: &str) -> Option<String> {
    let leaf = raw.rsplit(['/', '\\']).next()?;
    let safe: String = leaf.chars().filter(|c| !c.is_control()).take(255).collect();
    let safe = safe.trim();
    (!safe.is_empty() && !matches!(safe, "." | "..")).then(|| safe.to_string())
}

pub(crate) fn content_disposition(name: &str) -> Option<HeaderValue> {
    use std::fmt::Write;
    let name = sanitize_download_filename(name)?;
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_') {
            encoded.push(char::from(byte));
        } else {
            write!(&mut encoded, "%{byte:02X}").expect("writing into a string");
        }
    }
    HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{encoded}")).ok()
}

pub(crate) fn metadata_path(image: &Path) -> Option<PathBuf> {
    Some(
        image
            .parent()?
            .parent()?
            .join("attachment-names")
            .join(format!("{}.json", image.file_name()?.to_str()?)),
    )
}

pub(super) fn store(image: &Path, name: Option<&str>) -> Result<(), CliError> {
    let Some(name) = name.and_then(sanitize_download_filename) else {
        return Ok(());
    };
    let path = metadata_path(image).ok_or_else(filename_write_error)?;
    ensure_private_dir(path.parent().ok_or_else(filename_write_error)?)?;
    nils_common::fs::write_atomic(
        &path,
        &serde_json::to_vec(&name).map_err(|_| filename_write_error())?,
        0o600,
    )
    .map_err(|_| filename_write_error())
}

fn filename_write_error() -> CliError {
    CliError::runtime(
        "file-write-failed",
        "failed to store the attachment filename",
        None,
    )
}

pub(super) fn sanitize_attachment_filename(raw: &str) -> String {
    let leaf = Path::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment.bin");
    let mut safe = leaf
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    safe = safe
        .trim_matches(|ch| matches!(ch, '.' | '-' | '_'))
        .to_string();
    if safe.is_empty() {
        safe = "attachment.bin".to_string();
    }
    if safe.len() > 120 {
        safe.truncate(120);
    }
    safe
}

pub(super) fn attachment_candidate_path(dir: &Path, filename: &str, attempt: usize) -> PathBuf {
    let stamp = Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    if attempt == 0 {
        dir.join(format!("{stamp}-{filename}"))
    } else {
        dir.join(format!("{stamp}-{attempt}-{filename}"))
    }
}
