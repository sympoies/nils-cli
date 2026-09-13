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

fn replace_months_section(index: &str, months: &[Month]) -> String {
    let rendered = render_months(months);
    let Some(start) = index.find(&format!("{MONTHS_HEADING}\n")) else {
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

    let after_heading = start + MONTHS_HEADING.len() + 1;
    let tail = &index[after_heading..];
    // The section runs until the next `## ` heading at line start, or EOF.
    let end_offset = tail
        .match_indices("\n## ")
        .map(|(offset, _)| offset + 1)
        .next()
        .unwrap_or(tail.len());

    let mut out = String::with_capacity(index.len());
    out.push_str(&index[..after_heading]);
    out.push('\n');
    out.push_str(&rendered);
    out.push('\n');
    let remainder = &tail[end_offset..];
    if !remainder.is_empty() {
        out.push('\n');
        out.push_str(remainder.trim_start_matches('\n'));
    }
    out
}
