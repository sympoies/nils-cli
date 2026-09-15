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

/// The section labels an entry must carry.
///
/// `Links` is deliberately absent. It was required in the first cut of this
/// crate, inferred from a backfill whose entries were written in one pass and
/// all carried links. Measured against the logs that already existed across the
/// organization, that inference was wrong: of 483 hand-written entries, 60 have
/// no `Links` section, because the author had nothing worth linking. A required
/// section that real authors routinely and correctly omit is a wrong
/// requirement, not a widespread defect.
///
/// `Follow-ups` has always been optional for the same reason.
pub const REQUIRED_SECTIONS: [&str; 3] = ["Result", "Why / context", "Evidence"];

/// Whether `label` must appear in every entry.
pub fn is_required_section(label: &str) -> bool {
    REQUIRED_SECTIONS.contains(&label)
}

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
    /// `Links` and `Follow-ups` are optional and are omitted when they have no
    /// bullets. The three sections in `REQUIRED_SECTIONS` always render, with a
    /// `TODO` bullet when empty, because an entry missing one of them is the
    /// shape `check` reports.
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
            // An optional section with nothing in it is omitted rather than
            // rendered as a TODO: a placeholder the author chose not to fill is
            // noise in a log that exists to be read later. Required sections
            // still render their TODO, because a missing one is a real gap and
            // `check` reports it.
            if bullets.is_empty() && !is_required_section(label) {
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

    // Refuse before touching a file git could not merge. Inserting here would
    // stack a new entry on top of an unresolved conflict and make the result
    // look like ordinary content, which is harder to notice than the conflict
    // it buried.
    if let Some(line) = crate::model::first_conflict_marker(&existing) {
        return Err(DevlogError::ConflictMarkers {
            path: path.clone(),
            line,
        });
    }

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
///
/// [`autolink`] runs first, so the angle brackets it may add are counted
/// against the budget rather than pushing the line past it afterwards.
fn render_bullet(text: &str) -> String {
    let text = autolink(text);
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

/// Wrap every bare URL in `text` in angle brackets.
///
/// `MD034` ("bare URL used") is enabled in this workspace's Markdown lint
/// baseline alongside the `MD036` rule that decided the section headings, so a
/// bare URL in a generated entry blocks the commit of the file the CLI just
/// wrote. This is the same class of defect and belongs in the same place: the
/// renderer, not the caller.
///
/// A whitespace-delimited word is the unit, because that is what the existing
/// logs are made of. Across the 1385 entries in the organization's logs a URL
/// scheme is preceded by `](` 1220 times (an inline link), by `<` 454 times
/// (already an autolink), by a backtick 39 times (a code span) and by
/// whitespace 74 times — and only that last form is what `MD034` rejects. The
/// first two are skipped for free because such a word does not start with the
/// scheme. A code span is the one form that can put a bare-looking URL after
/// whitespace, so it is tracked; wrapping a URL inside one would change the
/// command the entry is quoting, and `MD034` exempts it anyway.
///
/// `MD034` also covers bare email addresses. One appears in those 1385
/// entries, and deciding whether a word is an address is guesswork in a way
/// that matching a scheme is not, so an address is left as written.
fn autolink(text: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    // A code span can run across several words, so this state lives outside
    // the loop rather than being decided per word.
    let mut in_code_span = false;

    for word in text.split_whitespace() {
        words.push(if in_code_span {
            word.to_string()
        } else {
            wrap_bare_url(word)
        });
        // A backtick run opens a span and the next one closes it, so an odd
        // number of backticks in this word flips the state.
        if word.bytes().filter(|byte| *byte == b'`').count() % 2 == 1 {
            in_code_span = !in_code_span;
        }
    }

    words.join(" ")
}

/// Trailing characters that end a sentence rather than a URL.
///
/// `rumdl fmt` ends a URL before these and leaves them outside the brackets;
/// matching it keeps what this CLI writes identical to what the formatter
/// would have rewritten it to. A closing parenthesis is deliberately absent
/// for the same reason `rumdl` keeps one: it is part of the URL often enough
/// (a Wikipedia title, say) that trimming it would break more links than it
/// tidied.
const SENTENCE_PUNCTUATION: [char; 7] = ['.', ',', ';', ':', '!', '?', ']'];

/// Wrap `word` in angle brackets when it is a bare URL, leaving any trailing
/// sentence punctuation outside them.
fn wrap_bare_url(word: &str) -> String {
    if !word.starts_with("https://") && !word.starts_with("http://") {
        return word.to_string();
    }
    let url = word.trim_end_matches(SENTENCE_PUNCTUATION);
    let trailing = &word[url.len()..];
    format!("<{url}>{trailing}")
}
