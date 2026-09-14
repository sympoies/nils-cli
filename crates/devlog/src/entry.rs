//! Entry rendering and newest-first insertion.
//!
//! The section labels are rendered as real `###` headings rather than bold
//! text. `MD036` ("emphasis used instead of a heading") is enabled in this
//! workspace's Markdown lint baseline, so a bold label fails the docs lane;
//! encoding that here is the difference between a CLI that produces
//! lint-clean entries and one that hands the author a broken file.

use std::fmt::Write as _;

use crate::model::{Devlog, DevlogError, EntryDate, Month};

/// The section labels an entry carries, in render order.
pub const SECTIONS: [&str; 5] = ["Result", "Why / context", "Evidence", "Links", "Follow-ups"];

/// A single devlog entry before it is rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub date: Option<EntryDate>,
    pub title: String,
    pub result: Vec<String>,
    pub why: Vec<String>,
    pub evidence: Vec<String>,
    pub links: Vec<String>,
    pub follow_ups: Vec<String>,
}

impl Entry {
    /// Render the entry body, starting at its `## YYYY-MM-DD - title` heading
    /// and ending with a single trailing newline.
    ///
    /// `Follow-ups` is optional per the entry template and is omitted when it
    /// has no bullets. The other four sections always render, because an entry
    /// missing them is the shape `check` reports.
    pub fn render(&self, date: EntryDate) -> String {
        let mut out = String::new();
        // Writing into a String is infallible; the `_ =` keeps the lint quiet
        // without introducing an unwrap that could read as a real failure path.
        _ = writeln!(out, "## {date} - {}", self.title.trim());

        let sections: [(&str, &Vec<String>); 5] = [
            ("Result", &self.result),
            ("Why / context", &self.why),
            ("Evidence", &self.evidence),
            ("Links", &self.links),
            ("Follow-ups", &self.follow_ups),
        ];

        for (label, bullets) in sections {
            if label == "Follow-ups" && bullets.is_empty() {
                continue;
            }
            _ = writeln!(out);
            _ = writeln!(out, "### {label}");
            _ = writeln!(out);
            if bullets.is_empty() {
                _ = writeln!(out, "{}", render_bullet("TODO"));
            } else {
                for bullet in bullets {
                    _ = writeln!(out, "{}", render_bullet(bullet));
                }
            }
        }

        out
    }
}

/// The outcome of inserting an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insertion {
    pub month: Month,
    pub date: EntryDate,
    /// True when the month file did not exist and was created.
    pub created_month_file: bool,
}

/// Insert `entry` at the top of its month file, creating the file with its
/// heading when absent.
///
/// Newest-first is positional, not sorted: the entry goes immediately below
/// the `# Development log - YYYY-MM` heading. An author back-dating an entry
/// is inserting where they asked to, and `check` reports the ordering rather
/// than this function silently re-sorting someone's file.
pub fn insert(devlog: &Devlog, entry: &Entry, date: EntryDate) -> Result<Insertion, DevlogError> {
    let month = date.month();
    let path = devlog.month_path(month);
    let heading = month.heading();
    let rendered = entry.render(date);

    let (existing, created) = match std::fs::read_to_string(&path) {
        Ok(contents) => (contents, false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (format!("{heading}\n"), true),
        Err(source) => {
            return Err(DevlogError::Io {
                path: path.clone(),
                source,
            });
        }
    };

    let mut lines = existing.lines();
    let first = lines.next().unwrap_or_default();
    if first.trim() != heading {
        return Err(DevlogError::MissingHeading {
            path: path.clone(),
            expected: heading,
        });
    }

    let rest = existing
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .trim_start_matches('\n');

    let mut updated = String::with_capacity(existing.len() + rendered.len() + 2);
    updated.push_str(first);
    updated.push('\n');
    updated.push('\n');
    updated.push_str(&rendered);
    if !rest.is_empty() {
        updated.push('\n');
        updated.push_str(rest);
    }

    std::fs::write(&path, updated).map_err(|source| DevlogError::Io {
        path: path.clone(),
        source,
    })?;

    Ok(Insertion {
        month,
        date,
        created_month_file: created,
    })
}

/// Column at which bullet text wraps.
///
/// The workspace Markdown lint caps lines at 140 characters (`MD013`), and the
/// existing entries wrap well inside that. Emitting unwrapped bullets would
/// make every generated entry fail the docs lane, which defeats the purpose of
/// generating them.
const WRAP_COLUMN: usize = 79;

/// Render one bullet as `- text` with two-space continuation lines, wrapped at
/// [`WRAP_COLUMN`].
///
/// Wrapping is whitespace-only: a single token longer than the budget (a long
/// URL, say) is emitted intact and allowed to overrun rather than being broken
/// into something that no longer resolves.
fn render_bullet(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    // The first line carries the list marker; every wrapped line that follows
    // is indented to align under it, which is the only continuation form.
    let mut current = String::from("- ");
    let mut has_word = false;

    for word in text.split_whitespace() {
        if has_word && current.chars().count() + 1 + word.chars().count() > WRAP_COLUMN {
            lines.push(current);
            current = format!("  {word}");
            has_word = true;
            continue;
        }
        if has_word {
            current.push(' ');
        }
        current.push_str(word);
        has_word = true;
    }
    lines.push(current);
    lines.join("\n")
}
