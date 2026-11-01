//! Exact Git tag selectors and the fixed canonical SemVer deletion guard.
use super::super::super::github_write::invalid;
use crate::error::ForgeError;

pub(super) fn tag(tag: &str, op: &str) -> Result<(), ForgeError> {
    let unsafe_tag = tag.is_empty()
        || tag.starts_with('-')
        || tag.starts_with("refs/")
        || tag.starts_with('/')
        || tag.ends_with('/')
        || tag.contains("..")
        || tag.contains("@{")
        || tag
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\%#".contains(c))
        || tag.split('/').any(|p| {
            p.is_empty() || p.starts_with('.') || p.ends_with('.') || p.ends_with(".lock")
        });
    if unsafe_tag {
        return Err(invalid(
            op,
            "release_tag_invalid",
            "supply one literal Git tag name, without refs/ prefix, revision syntax, URL escapes or options",
        ));
    }
    Ok(())
}

fn number(part: &str) -> bool {
    !part.is_empty()
        && part.bytes().all(|b| b.is_ascii_digit())
        && (part == "0" || !part.starts_with('0'))
}
fn identifiers(value: &str, prerelease: bool) -> bool {
    value.split('.').all(|part| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && (!prerelease || !part.bytes().all(|b| b.is_ascii_digit()) || number(part))
    })
}
/// SemVer 2.0.0 grammar, with one optional lowercase v; no numeric range cap.
pub(super) fn version_tag(tag: &str) -> bool {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let core = if let Some((version, build)) = tag.split_once('+') {
        if !identifiers(build, false) {
            return false;
        }
        version
    } else {
        tag
    };
    let core = if let Some((version, pre)) = core.split_once('-') {
        if !identifiers(pre, true) {
            return false;
        }
        version
    } else {
        core
    };
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3 && parts.into_iter().all(number)
}

pub(super) fn encode(tag: &str) -> String {
    url::form_urlencoded::byte_serialize(tag.as_bytes()).collect()
}
