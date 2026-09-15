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
/// The scan is over the whole bullet rather than over whitespace-delimited
/// words, because the two contexts that must be preserved do not respect word
/// boundaries. Across the 1385 entries in the organization's logs a URL scheme
/// is preceded by `](` 1220 times (an inline link target), by `<` 454 times
/// (already an autolink), by a backtick 39 times (a code span) and by
/// whitespace 74 times — and only that last form is what `MD034` rejects. A
/// URL inside a code span is exempt from `MD034` already, and wrapping it
/// would change the command the entry is quoting, so a span is copied through
/// whole.
///
/// `MD034` also covers bare email addresses. One appears in those 1385
/// entries, and deciding whether a word is an address is guesswork in a way
/// that matching a scheme is not, so an address is left as written.
fn autolink(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    // Everything before `copied` is already in `out`; the scan emits slices
    // rather than bytes so multi-byte characters pass through intact.
    let mut copied = 0usize;
    let mut at = 0usize;

    while at < bytes.len() {
        match bytes[at] {
            b'`' => {
                let run = backtick_run(bytes, at);
                match code_span_end(bytes, at + run, run) {
                    // A code span, copied through untouched.
                    Some(end) => at = end,
                    // An unmatched run is literal text and opens nothing, so
                    // the scan continues. Treating it as an opener would
                    // silently stop autolinking for the rest of the bullet.
                    None => at += run,
                }
            }
            b'h' => match bare_url_end(text, at) {
                Some(end) => {
                    out.push_str(&text[copied..at]);
                    out.push('<');
                    out.push_str(&text[at..end]);
                    out.push('>');
                    copied = end;
                    at = end;
                }
                None => at += 1,
            },
            // Advancing a byte at a time is safe because every byte matched
            // above is ASCII, and an ASCII byte never occurs inside a
            // multi-byte character.
            _ => at += 1,
        }
    }

    out.push_str(&text[copied..]);
    out
}

/// The length of the run of backticks starting at `at`.
fn backtick_run(bytes: &[u8], at: usize) -> usize {
    bytes[at..].iter().take_while(|byte| **byte == b'`').count()
}

/// Where the code span opened by a run of `run` backticks ends, past its
/// closing run.
///
/// A span closes on a run of exactly the same length, per CommonMark — a
/// longer or shorter run is content. `model.rs` draws the same distinction for
/// fences, for the same reason: counting backticks rather than runs gets
/// ```` ``code`` ```` wrong in both directions.
fn code_span_end(bytes: &[u8], from: usize, run: usize) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let closing = backtick_run(bytes, at);
        if closing == run {
            return Some(at + closing);
        }
        at += closing;
    }
    None
}

/// Trailing characters that end a sentence rather than a URL.
///
/// `rumdl fmt` ends a URL before each of these and leaves it outside the
/// brackets, and so does this. That is a statement about this set, not a claim
/// of parity with the formatter: `rumdl` also ends a URL at a quote character,
/// which this does not, so the two disagree on a quoted URL.
const SENTENCE_PUNCTUATION: [char; 7] = ['.', ',', ';', ':', '!', '?', ']'];

/// Where the bare URL starting at `at` ends, or `None` when there is not one
/// there to wrap.
///
/// A URL that is already part of a link is left alone, whichever half of the
/// link it is. Those forms carry 1674 of the 1787 URLs in the existing logs,
/// and rewriting one is worse than leaving a bare URL bare: a bare URL fails a
/// lint that says so, while a mangled link passes every lint and is simply
/// gone.
///
/// Only `http://` and `https://` are recognized. `MD034` also reports
/// `www.`-style hosts, other schemes and bare email addresses, so this does not
/// make a generated entry unconditionally lint-clean — it covers the forms that
/// actually appear in these logs.
fn bare_url_end(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let scheme = ["https://", "http://"]
        .into_iter()
        .find(|scheme| text[at..].starts_with(scheme))?;

    if at > 0 {
        let before = bytes[at - 1];
        // `<url>` is already an autolink.
        if before == b'<' {
            return None;
        }
        // `[url](dest)` and `[url][1]` use the URL as the link text. The scan
        // below has no bracket terminator, so wrapping here would run through
        // `](` and swallow the destination, turning a link that every lint
        // accepts into text that is not a link at all.
        if before == b'[' {
            return None;
        }
        // `](url)` is already an inline link target. A bare `(` is not: a
        // parenthesized URL in prose is still bare, and `rumdl` reports it.
        if before == b'(' && at >= 2 && bytes[at - 2] == b']' {
            return None;
        }
    }

    let mut end = at + scheme.len();
    while end < bytes.len()
        && !bytes[end].is_ascii_whitespace()
        && !matches!(bytes[end], b'<' | b'>' | b'`')
    {
        end += 1;
    }

    let mut url = &text[at..end];
    loop {
        url = url.trim_end_matches(SENTENCE_PUNCTUATION);
        // A closing parenthesis ends the URL only when the URL has no opener
        // for it. `rumdl fmt` draws the line in the same place, and it is the
        // line that keeps a Wikipedia-style title whole without swallowing the
        // parenthesis a sentence put around the link.
        if !(url.ends_with(')') && url.matches(')').count() > url.matches('(').count()) {
            break;
        }
        url = &url[..url.len() - 1];
    }

    // A scheme with nothing after it is not a URL worth linking.
    (url.len() > scheme.len()).then(|| at + url.len())
}
