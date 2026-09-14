//! The month index in the log's `README.md`.
//!
//! The index is a `## Months` list of links, newest first. It is the surface
//! that drifts silently: adding a month file does not add its link, and the
//! omission fails no audit.

use crate::model::{Devlog, DevlogError, Month};

const MONTHS_HEADING: &str = "## Months";

/// Every month the index currently links, in the order listed.
pub fn listed_months(index: &str) -> Vec<Month> {
    let mut months = Vec::new();
    let mut in_section = false;
    for line in index.lines() {
        if line.starts_with("## ") {
            in_section = line.trim() == MONTHS_HEADING;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some(rest) = line.trim().strip_prefix("- [") else {
            continue;
        };
        let Some((label, _)) = rest.split_once(']') else {
            continue;
        };
        if let Ok(month) = label.parse::<Month>() {
            months.push(month);
        }
    }
    months
}

/// The rendered `## Months` list for `months`, newest first.
pub fn render_months(months: &[Month]) -> String {
    let mut sorted = months.to_vec();
    sorted.sort_by(|a, b| b.cmp(a));
    sorted
        .iter()
        .map(|month| format!("- [{month}]({month}.md)"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The outcome of regenerating the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexUpdate {
    pub changed: bool,
    pub months: Vec<Month>,
}

/// Fail if the index still holds an unresolved merge conflict.
///
/// `new` mutates the month file and then the index. Checking only inside
/// `sync` would let the month file be written before the index refused, so a
/// caller that retries after resolving the conflict would insert the entry
/// twice. Callers that mutate both files check this first.
pub fn assert_resolved(devlog: &Devlog) -> Result<(), DevlogError> {
    let path = devlog.index_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        // A missing index is not this function's failure to report; `sync`
        // and `check` each say something more useful about it.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(DevlogError::Io { path, source }),
    };
    match crate::model::first_conflict_marker(&contents) {
        Some(line) => Err(DevlogError::ConflictMarkers { path, line }),
        None => Ok(()),
    }
}

/// Rewrite the index's `## Months` section from the tracked month files.
///
/// Only that section is touched: everything a repository wrote around it is
/// preserved, because the index is also the log's conventions document.
pub fn sync(devlog: &Devlog) -> Result<IndexUpdate, DevlogError> {
    let scan = devlog.months()?;
    let path = devlog.index_path();
    let contents = std::fs::read_to_string(&path).map_err(|source| DevlogError::Io {
        path: path.clone(),
        source,
    })?;

    // Refuse before rewriting a file git could not merge. Regenerating the
    // `## Months` section of a conflicted index would resolve part of the
    // conflict and leave the rest, which reads as an ordinary file and hides
    // that a human still has to finish the merge. `new` has already checked
    // this through `assert_resolved`; `devlog index` reaches it here.
    if let Some(line) = crate::model::first_conflict_marker(&contents) {
        return Err(DevlogError::ConflictMarkers {
            path: path.clone(),
            line,
        });
    }

    let updated = replace_months_section(&contents, &scan.months);
    let changed = updated != contents;
    if changed {
        std::fs::write(&path, &updated).map_err(|source| DevlogError::Io {
            path: path.clone(),
            source,
        })?;
    }

    let mut months = scan.months;
    months.sort_by(|a, b| b.cmp(a));
    Ok(IndexUpdate { changed, months })
}

/// Byte range of the `## Months` section body, and the offset just past its
/// heading line.
///
/// The heading is matched line-wise, not by substring: a substring search also
/// matches `### Months` (which contains `## Months` at offset 1) and misses a
/// heading that ends the file without a trailing newline. `listed_months`
/// anchors on line starts, so matching the same way keeps `index` and `check`
/// from disagreeing about the same file.
fn months_section(index: &str) -> Option<(usize, usize)> {
    let mut offset = 0usize;
    let mut heading_end: Option<usize> = None;

    for line in index.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if heading_end.is_none() {
            if trimmed == MONTHS_HEADING {
                heading_end = Some(offset + line.len());
            }
        } else if trimmed.starts_with("## ") {
            return heading_end.map(|start| (start, offset));
        }
        offset += line.len();
    }

    heading_end.map(|start| (start, index.len()))
}

fn replace_months_section(index: &str, months: &[Month]) -> String {
    let rendered = render_months(months);
    let Some((body_start, body_end)) = months_section(index) else {
        // No section yet: append one rather than failing, so a log created by
        // hand can be brought under the contract by running `devlog index`.
        let mut out = index.trim_end().to_string();
        out.push_str("\n\n");
        out.push_str(MONTHS_HEADING);
        out.push_str("\n\n");
        out.push_str(&rendered);
        out.push('\n');
        return out;
    };

    let mut out = String::with_capacity(index.len());
    out.push_str(&index[..body_start]);
    // `body_start` sits just past the heading line, which may itself have had
    // no trailing newline at end of file.
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&rendered);
    out.push('\n');

    let remainder = index[body_end..].trim_start_matches('\n');
    if !remainder.is_empty() {
        out.push('\n');
        out.push_str(remainder);
    }
    out
}
