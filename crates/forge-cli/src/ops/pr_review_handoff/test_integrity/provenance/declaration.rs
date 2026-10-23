use serde::{Deserialize, Serialize};

use super::{ForgeError, refusal};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Declaration {
    pub repository: String,
    pub commit: String,
    pub mappings: Vec<Mapping>,
    pub adaptations: Vec<String>,
    #[serde(default)]
    pub additions: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mapping {
    pub destination: String,
    pub source: String,
}

fn invalid() -> ForgeError {
    refusal(
        "provenance_import_invalid",
        "invalid provenance import declaration",
        "Use one ## Provenance import section with repository owner/name, a full commit SHA, non-overlapping directory mappings and exact adaptation/addition paths. See crates/forge-cli/docs/specs/test-change-admission-v1.md#provenance-imports.",
    )
}

pub(crate) fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.is_ascii()
        && !path.bytes().any(|b| {
            b.is_ascii_whitespace() || b.is_ascii_control() || matches!(b, b'"' | b'|' | b'\\')
        })
        && path.split('/').all(|p| !matches!(p, "" | "." | ".."))
}

fn prefix(path: &str) -> bool {
    path.strip_suffix('/').is_some_and(valid_path)
}

/// Headings in examples cannot activate the mode. The section itself may use
/// one JSON code fence; duplicate sections and trailing prose fail closed.
pub(crate) fn parse(body: &str) -> Result<Option<Declaration>, ForgeError> {
    let mut active = false;
    let mut found = false;
    let mut fence = None;
    let mut lines = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if fence.is_none() && line.starts_with("## ") {
            active = trimmed == "## Provenance import";
            if active {
                if found {
                    return Err(invalid());
                }
                found = true;
            }
            continue;
        }
        if active {
            lines.push(line);
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = &trimmed[..3];
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
        }
    }
    if !found {
        return Ok(None);
    }
    let section = lines.join("\n");
    let mut text = section.trim();
    if let Some(rest) = text
        .strip_prefix("```json\n")
        .or_else(|| text.strip_prefix("```\n"))
    {
        text = rest.strip_suffix("```").ok_or_else(invalid)?.trim();
    }
    let d: Declaration = serde_json::from_str(text).map_err(|_| invalid())?;
    let repo: Vec<_> = d.repository.split('/').collect();
    let repo_part = |p: &&str| {
        !p.is_empty()
            && !matches!(*p, "." | "..")
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if repo.len() != 2
        || !repo.iter().all(repo_part)
        || !super::super::super::valid_sha(&d.commit)
        || d.mappings.is_empty()
        || d.mappings
            .iter()
            .any(|m| !prefix(&m.destination) || (!m.source.is_empty() && !prefix(&m.source)))
        || d.adaptations.iter().any(|p| !valid_path(p))
        || d.additions.iter().any(|p| {
            !valid_path(p)
                || d.adaptations.contains(p)
                || !d.mappings.iter().any(|m| p.starts_with(&m.destination))
        })
    {
        return Err(invalid());
    }
    for (i, mapping) in d.mappings.iter().enumerate() {
        if d.mappings.iter().skip(i + 1).any(|m| {
            m.destination.starts_with(&mapping.destination)
                || mapping.destination.starts_with(&m.destination)
        }) {
            return Err(invalid());
        }
    }
    let unique: std::collections::BTreeSet<_> = d.adaptations.iter().collect();
    let additions: std::collections::BTreeSet<_> = d.additions.iter().collect();
    if unique.len() != d.adaptations.len() || additions.len() != d.additions.len() {
        return Err(invalid());
    }
    Ok(Some(d))
}
