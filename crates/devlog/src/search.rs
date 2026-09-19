//! Literal, case-insensitive search across month files.

use serde::Serialize;

use crate::model::{Devlog, DevlogError, Month, first_conflict_marker};

/// One matching line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Match {
    pub month: String,
    pub line_number: usize,
    pub line: String,
}

/// An unresolved merge conflict observed while searching a month file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchConflict {
    pub month: String,
    pub line_number: usize,
}

impl SearchConflict {
    /// Render the non-fatal diagnostic shared by text and JSON output.
    pub fn warning(&self) -> String {
        format!(
            "note: {}.md:{}: unresolved merge conflict; search results may include both sides",
            self.month, self.line_number
        )
    }
}

/// The result of a search.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchReport {
    pub term: String,
    pub months_searched: usize,
    pub matches: Vec<Match>,
    #[serde(skip_serializing)]
    pub conflicts: Vec<SearchConflict>,
}

/// Search `term` across every month file, or one month when `month` is given.
///
/// Matching is literal and case-insensitive: the terms people look up are
/// crate names, flags, and error codes, which regex metacharacters would
/// mangle rather than help.
pub fn search(
    devlog: &Devlog,
    term: &str,
    month: Option<Month>,
) -> Result<SearchReport, DevlogError> {
    let months = match month {
        Some(month) => {
            let path = devlog.month_path(month);
            if !path.is_file() {
                return Err(DevlogError::MissingMonthFile { path });
            }
            vec![month]
        }
        None => devlog.months()?.months,
    };

    let needle = term.to_lowercase();
    let mut matches = Vec::new();
    let mut conflicts = Vec::new();

    for month in &months {
        let path = devlog.month_path(*month);
        let contents = std::fs::read_to_string(&path).map_err(|source| DevlogError::Io {
            path: path.clone(),
            source,
        })?;
        if let Some(line_number) = first_conflict_marker(&contents) {
            conflicts.push(SearchConflict {
                month: month.to_string(),
                line_number,
            });
        }
        for (index, line) in contents.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                matches.push(Match {
                    month: month.to_string(),
                    line_number: index + 1,
                    line: line.to_string(),
                });
            }
        }
    }

    Ok(SearchReport {
        term: term.to_string(),
        months_searched: months.len(),
        matches,
        conflicts,
    })
}
