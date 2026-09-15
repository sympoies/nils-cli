//! Structural integrity checks.
//!
//! This is the surface that has no owner when a devlog is a directory plus a
//! shell search helper: a mis-named month file is silently skipped by a glob,
//! and an index that drifts from the tracked files fails no audit.

use std::path::Path;

use serde::Serialize;

use crate::entry::{REQUIRED_SECTIONS, SECTIONS};
use crate::model::{Devlog, DevlogError, Month};

/// One structural problem found in the log.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Problem {
    /// Stable machine-readable class.
    pub kind: &'static str,
    /// Repository-relative path, forward-slashed.
    pub path: String,
    /// Human-readable description.
    pub detail: String,
}

/// The result of a full check.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckReport {
    pub devlog_dir: String,
    pub month_count: usize,
    pub entry_count: usize,
    pub problems: Vec<Problem>,
}

impl CheckReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub fn check(devlog: &Devlog) -> Result<CheckReport, DevlogError> {
    let scan = devlog.months()?;
    let mut problems = Vec::new();
    let mut entry_count = 0usize;

    for path in &scan.unexpected {
        problems.push(Problem {
            kind: "unexpected-file",
            path: relative(devlog, path),
            detail: "not a YYYY-MM.md month file; it is invisible to search and to the index"
                .to_string(),
        });
    }

    for month in &scan.months {
        let path = devlog.month_path(*month);
        let contents = std::fs::read_to_string(&path).map_err(|source| DevlogError::Io {
            path: path.clone(),
            source,
        })?;
        let month_problems = check_month(devlog, *month, &contents, &mut entry_count);
        problems.extend(month_problems);
    }

    problems.extend(check_index(devlog, &scan.months)?);

    Ok(CheckReport {
        devlog_dir: devlog.relative_dir(),
        month_count: scan.months.len(),
        entry_count,
        problems,
    })
}

fn check_month(
    devlog: &Devlog,
    month: Month,
    contents: &str,
    entry_count: &mut usize,
) -> Vec<Problem> {
    let path = devlog.month_path(month);
    let relative = relative(devlog, &path);
    let mut problems = Vec::new();

    // Report a conflict and stop parsing this file. Both sides of an
    // unresolved conflict are syntactically valid entries, so continuing would
    // count them as real and report a clean structure for a file that cannot
    // be published.
    if let Some(line) = crate::model::first_conflict_marker(contents) {
        problems.push(Problem {
            kind: "conflict-markers",
            path: relative,
            detail: format!(
                "unresolved merge conflict at line {line}; both sides parse as entries, so nothing below here is trustworthy"
            ),
        });
        return problems;
    }

    let heading = month.heading();
    let first = contents.lines().next().unwrap_or_default().trim();
    if first != heading {
        problems.push(Problem {
            kind: "missing-heading",
            path: relative.clone(),
            detail: format!("expected the file to open with '{heading}', found '{first}'"),
        });
    }

    let mut dates: Vec<String> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut finished: Vec<(String, Vec<String>)> = Vec::new();

    // A heading inside a fenced block is an example, not structure. The
    // conflict scan already excludes fenced markers for this reason, and an
    // entry documenting this very format is the case both exclusions exist
    // for: reading its quoted `## 2026-04-17 - Title` as a real entry would
    // report a malformed heading and three missing sections that nobody can
    // repair without editing the prose.
    let mut fences = crate::model::FenceScanner::default();
    for line in contents.lines() {
        if !fences.is_structural(line) {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(entry) = current.take() {
                finished.push(entry);
            }
            let title = rest.trim().to_string();
            let date = title
                .split_once(" - ")
                .map(|(date, _)| date.trim().to_string());
            match date {
                Some(date) => dates.push(date),
                None => problems.push(Problem {
                    kind: "malformed-entry-heading",
                    path: relative.clone(),
                    detail: format!("entry heading '## {title}' is not '## YYYY-MM-DD - <title>'"),
                }),
            }
            current = Some((title, Vec::new()));
            *entry_count += 1;
        } else if let Some(label) = line.strip_prefix("### ")
            && let Some((_, labels)) = current.as_mut()
        {
            labels.push(label.trim().to_string());
        }
    }
    if let Some(entry) = current.take() {
        finished.push(entry);
    }

    for (title, labels) in &finished {
        for required in &REQUIRED_SECTIONS {
            if !labels.iter().any(|label| label == required) {
                problems.push(Problem {
                    kind: "missing-section",
                    path: relative.clone(),
                    detail: format!("entry '{title}' has no '### {required}' section"),
                });
            }
        }
        for label in labels {
            if !SECTIONS.contains(&label.as_str()) {
                problems.push(Problem {
                    kind: "unknown-section",
                    path: relative.clone(),
                    detail: format!("entry '{title}' has an unrecognized section '### {label}'"),
                });
            }
        }
    }

    for date in &dates {
        if !date_belongs_to(date, month) {
            problems.push(Problem {
                kind: "date-month-mismatch",
                path: relative.clone(),
                detail: format!("entry dated '{date}' does not belong to {month}"),
            });
        }
    }

    let mut sorted = dates.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    if sorted != dates {
        problems.push(Problem {
            kind: "not-newest-first",
            path: relative,
            detail: "entries are not ordered newest-first".to_string(),
        });
    }

    problems
}

fn date_belongs_to(date: &str, month: Month) -> bool {
    date.strip_suffix(|c: char| c.is_ascii_digit())
        .and_then(|rest| rest.strip_suffix(|c: char| c.is_ascii_digit()))
        .and_then(|rest| rest.strip_suffix('-'))
        .is_some_and(|prefix| prefix == month.to_string())
}

fn check_index(devlog: &Devlog, months: &[Month]) -> Result<Vec<Problem>, DevlogError> {
    let path = devlog.index_path();
    let relative = relative(devlog, &path);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![Problem {
                kind: "missing-index",
                path: relative,
                detail: "the log has no README.md index".to_string(),
            }]);
        }
        Err(source) => return Err(DevlogError::Io { path, source }),
    };

    // Both sides of an unresolved conflict list months, so comparing the merged
    // parse against the month files would report a plausible index for a file
    // that cannot be published.
    if let Some(line) = crate::model::first_conflict_marker(&contents) {
        return Ok(vec![Problem {
            kind: "conflict-markers",
            path: relative,
            detail: format!(
                "unresolved merge conflict at line {line}; the month list below it cannot be trusted"
            ),
        }]);
    }

    let listed = crate::index::listed_months(&contents);
    let mut problems = Vec::new();

    for month in months {
        if !listed.contains(month) {
            problems.push(Problem {
                kind: "index-missing-month",
                path: relative.clone(),
                detail: format!("{month} has a month file but is not listed in the index"),
            });
        }
    }
    for month in &listed {
        if !months.contains(month) {
            problems.push(Problem {
                kind: "index-stale-month",
                path: relative.clone(),
                detail: format!("the index lists {month} but no such month file exists"),
            });
        }
    }

    Ok(problems)
}
fn relative(devlog: &Devlog, path: &Path) -> String {
    devlog.relative(path)
}
