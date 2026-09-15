//! Mechanical repairs that bring an existing log to the shape `check` accepts.
//!
//! Every log in this organization was written by hand before this crate
//! existed, and they drifted from the conventions in the same few ways:
//! section labels written as bold text rather than headings, an em dash where
//! the parser splits on a hyphen, a month heading that disagrees with its
//! filename, entries out of order, and required sections that predate the
//! requirement.
//!
//! Each of those has exactly one correct repair, which is why they belong to a
//! command rather than to whoever is holding the log that day. What has no
//! single correct repair — a file that is not a month, an entry heading with
//! no readable date, a section outside the template, a date in the wrong month
//! — is left alone and reported, because repairing it would mean inventing
//! what the author meant.
//!
//! There is no dry run. `check` is the read-only question and already answers
//! it; a second command reporting a different view of the same log would be
//! one more thing to keep honest.

use serde::Serialize;

use crate::check::Problem;
use crate::entry::{REQUIRED_SECTIONS, SECTIONS};
use crate::index;
use crate::model::{Devlog, DevlogError, EntryDate, Month};

/// The bullet a backfilled section carries.
///
/// It records the absence and names what added it, and stops there. Inventing
/// a plausible `Result` for an entry whose author never wrote one would put a
/// false claim in a log that exists to be trusted later — and so would the
/// earlier wording, which asserted that the entry predated the section
/// contract. Nothing checks that. An entry written yesterday and missing a
/// section gets this bullet too, and in a repository whose log is an audit
/// record, a tool-authored claim about *why* evidence is absent is exactly the
/// kind of content that must not be invented.
const PLACEHOLDER: &str = "- Not recorded; added by `devlog fix`.";

/// The dashes that appear where the parser expects `-`.
const DASHES: [char; 2] = ['\u{2014}', '\u{2013}'];

/// What `fix` changed, by kind.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Repairs {
    /// `**Result**` rewritten as `### Result`.
    pub section_labels: usize,
    /// An em or en dash in a heading replaced with the `-` the parser splits on.
    pub heading_separators: usize,
    /// A month heading brought back into agreement with its filename.
    pub month_headings: usize,
    /// Entries moved to restore newest-first order.
    pub reordered_entries: usize,
    /// Required sections added with [`PLACEHOLDER`].
    pub backfilled_sections: usize,
}

impl Repairs {
    pub fn total(&self) -> usize {
        self.section_labels
            + self.heading_separators
            + self.month_headings
            + self.reordered_entries
            + self.backfilled_sections
    }

    fn add(&mut self, other: Self) {
        self.section_labels += other.section_labels;
        self.heading_separators += other.heading_separators;
        self.month_headings += other.month_headings;
        self.reordered_entries += other.reordered_entries;
        self.backfilled_sections += other.backfilled_sections;
    }
}

/// The result of a full repair.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FixReport {
    pub devlog_dir: String,
    pub files_changed: usize,
    pub repairs: Repairs,
    pub index_updated: bool,
    /// What `check` still reports once the repairs are written.
    pub remaining: Vec<Problem>,
}

impl FixReport {
    pub fn ok(&self) -> bool {
        self.remaining.is_empty()
    }
}

/// Repair every month file, then the index, and report what `check` still sees.
pub fn fix(devlog: &Devlog) -> Result<FixReport, DevlogError> {
    // Refuse before rewriting anything git could not merge, for the reason
    // `new` and `index` refuse: a repaired log sitting beside an unresolved
    // conflict reads as an ordinary file, and the repairs would make the
    // conflict harder to notice rather than easier.
    index::assert_resolved(devlog)?;
    let scan = devlog.months()?;
    for month in &scan.months {
        let path = devlog.month_path(*month);
        let contents = read(&path)?;
        if let Some(line) = crate::model::first_conflict_marker(&contents) {
            return Err(DevlogError::ConflictMarkers { path, line });
        }
    }

    let mut repairs = Repairs::default();
    let mut files_changed = 0usize;

    for month in &scan.months {
        let path = devlog.month_path(*month);
        let contents = read(&path)?;
        let (repaired, file_repairs) = repair_month(&contents, *month);
        repairs.add(file_repairs);
        if repaired != contents {
            files_changed += 1;
            std::fs::write(&path, &repaired).map_err(|source| DevlogError::Io {
                path: path.clone(),
                source,
            })?;
        }
    }

    // The index lists the month files, so it is rewritten after them, the way
    // `new` pairs the two.
    //
    // A log with no index at all is left without one. The index is also the
    // log's conventions document, so creating it would be authoring content
    // rather than repairing structure, and whether a repository has a log is
    // its owner's decision rather than this command's. `check` reports the
    // absence, and it travels out in `remaining`.
    //
    // A symlinked index is not written through, for the reason `months()`
    // refuses a symlinked month file: the rewrite would land outside the log
    // and the link would look untouched in review. `devlog index` still
    // follows it, which is its own decision to make; this command rewrites
    // everything at once and is the wrong place to discover that.
    let index = devlog.index_path();
    let index_is_a_regular_file = std::fs::symlink_metadata(&index)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false);
    let index_updated = if index_is_a_regular_file {
        index::sync(devlog)?.changed
    } else {
        false
    };

    Ok(FixReport {
        devlog_dir: devlog.relative_dir(),
        files_changed,
        repairs,
        index_updated,
        remaining: crate::check::check(devlog)?.problems,
    })
}

fn read(path: &std::path::Path) -> Result<String, DevlogError> {
    std::fs::read_to_string(path).map_err(|source| DevlogError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Which lines of `lines` carry structure.
fn structural(lines: &[String]) -> Vec<bool> {
    crate::model::structural_line_mask(lines.iter().map(String::as_str))
}

/// Apply every repair to one month file's contents.
///
/// Returns the contents unchanged, and no repairs, when the file mixes line
/// endings. Rebuilding from `lines()` drops each `\r`, so one ending has to be
/// chosen for the whole file, and choosing rewrites every line that used the
/// other one — a whole-file diff from a command reporting that it repaired
/// nothing. Which ending a mixed file *meant* is not something this can know,
/// so it is left alone and its problems are reported instead.
fn repair_month(contents: &str, month: Month) -> (String, Repairs) {
    let repairs = Repairs::default();
    let crlf = contents.matches("\r\n").count();
    let lf = contents.matches('\n').count() - crlf;
    if crlf > 0 && lf > 0 {
        return (contents.to_string(), repairs);
    }
    let newline = if crlf > 0 { "\r\n" } else { "\n" };

    let trailing_newline = contents.ends_with('\n');
    let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();
    let mut repairs = repairs;

    // A line inside a fenced block is an example, not structure. An entry
    // documenting this very format is the obvious case, and `check` excludes
    // fenced conflict markers for the same reason; rewriting a label or a
    // heading there would edit the example rather than the entry.
    //
    // A label is promoted only where it is standing in for a section: inside
    // an entry, and only when that entry has no heading for it already. A
    // label before the first entry belongs to no entry, and a second one
    // inside an entry that already has the heading would make a duplicate
    // heading — which the lint reports and cannot fix, so promoting it would
    // repair one violation by introducing a worse one.
    let structural_lines = structural(&lines);
    let mut owner: Option<usize> = None;
    let mut present: Vec<&'static str> = Vec::new();
    for at in 0..lines.len() {
        if !structural_lines[at] {
            continue;
        }
        // The separator repair runs before anything classifies the line. A
        // `## 2026-04-17 — Title` is an entry heading either way, and deciding
        // that first meant its dash was never repaired.
        if let Some(repaired) = hyphenated_heading(&lines[at]) {
            lines[at] = repaired;
            repairs.heading_separators += 1;
        }

        if lines[at].starts_with("## ") {
            owner = Some(at);
            present.clear();
        } else if let Some(label) = lines[at]
            .strip_prefix("### ")
            .map(str::trim)
            .and_then(|label| SECTIONS.into_iter().find(|section| *section == label))
        {
            present.push(label);
        } else if let Some(label) = bold_section_label(&lines[at])
            && owner.is_some()
            && !present.contains(&label)
        {
            lines[at] = format!("### {label}");
            present.push(label);
            repairs.section_labels += 1;
        }
    }

    repair_month_heading(&mut lines, month, &mut repairs);
    reorder_entries(&mut lines, &mut repairs);
    backfill_sections(&mut lines, &mut repairs);

    let mut out = lines.join(newline);
    if trailing_newline {
        out.push_str(newline);
    }
    (out, repairs)
}

/// The section a line writes as a standalone bold label, if any.
///
/// Only the five template labels, and only when the bold span is the whole
/// line. Prose that happens to be bold inside an entry is not a section the
/// parser knows, and promoting it would invent structure the author did not
/// write.
///
/// The match is exact because the near misses are real. Of the 3136 standalone
/// bold lines in the organization's logs, 3105 are one of the five labels and
/// 31 are prose — and that 31 includes `**Why**`, `**Follow-up**` and
/// `**Why / root cause**`, each of which a prefix or fuzzy match would have
/// turned into a section its author never wrote.
///
/// The label must also start at column zero. An indented one is inside a list
/// item or an indented code block — neither is a section, and the fence mask
/// does not model indented blocks. All 3119 labels in those logs are at column
/// zero, so requiring it costs nothing and stops a four-space example from
/// being promoted and un-indented.
fn bold_section_label(line: &str) -> Option<&'static str> {
    let inner = line.trim_end().strip_prefix("**")?.strip_suffix("**")?;
    SECTIONS.into_iter().find(|section| *section == inner)
}

/// A heading whose two halves are separated by an em or en dash, rewritten
/// with the `-` the parser splits on.
///
/// Only the separator position is touched, so a dash inside a title survives.
/// Both forms confirm what they parsed before rewriting, and that guard is not
/// theoretical: the one heading in the organization's logs that carries a dash
/// today is `## 2026-06-21 - ... (v1.3.4–v1.3.7)`, already correctly separated.
/// Splitting it on its first dash yields something that is not a date, which is
/// exactly why the date is parsed before anything is rewritten.
fn hyphenated_heading(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("# Development log ") {
        let month = rest.strip_prefix(DASHES)?.trim();
        month.parse::<Month>().ok()?;
        return Some(format!("# Development log - {month}"));
    }

    let (date, title) = line.strip_prefix("## ")?.split_once(DASHES)?;
    let date = date.trim();
    date.parse::<EntryDate>().ok()?;
    Some(format!("## {date} - {}", title.trim()))
}

/// Bring a month heading back into agreement with its filename.
///
/// The filename is what `check` trusts and what `search` and the index are
/// keyed on, so a disagreement is repaired in the heading. A file that does
/// not open with a heading at all is left alone: that is a different problem,
/// and `check` reports it.
fn repair_month_heading(lines: &mut [String], month: Month, repairs: &mut Repairs) {
    let heading = month.heading();
    if let Some(first) = lines.first_mut()
        && first.starts_with("# ")
        && first.trim() != heading
    {
        *first = heading;
        repairs.month_headings += 1;
    }
}

/// Where each entry starts, in file order.
///
/// Fenced blocks are skipped, by the same scanner `check` reads them with. If
/// the two disagreed about what an entry is, `check` would report problems in
/// an example that `fix` could not see, and `fix` would splice a backfilled
/// section into the middle of a code block.
fn entry_starts(lines: &[String]) -> Vec<usize> {
    let structural_lines = structural(lines);
    lines
        .iter()
        .enumerate()
        .filter(|(at, line)| structural_lines[*at] && line.starts_with("## "))
        .map(|(at, _)| at)
        .collect()
}

/// The date an entry heading carries, if it is readable.
fn entry_date(heading: &str) -> Option<&str> {
    let (date, _) = heading.strip_prefix("## ")?.split_once(" - ")?;
    let date = date.trim();
    date.parse::<EntryDate>().ok()?;
    Some(date)
}

/// Split the lines into what precedes the first entry and one block per entry.
fn split_entries(lines: &[String]) -> (Vec<String>, Vec<Vec<String>>) {
    let starts = entry_starts(lines);
    let Some(&first) = starts.first() else {
        return (lines.to_vec(), Vec::new());
    };
    let mut blocks = Vec::with_capacity(starts.len());
    for (position, &start) in starts.iter().enumerate() {
        let end = starts.get(position + 1).copied().unwrap_or(lines.len());
        blocks.push(lines[start..end].to_vec());
    }
    (lines[..first].to_vec(), blocks)
}

/// Restore newest-first order, stably.
///
/// Stable means an entry already in the right place does not move, and entries
/// sharing a date keep the order their author gave them — that order carries
/// the within-day sequence the date alone cannot.
///
/// A file with any unreadable entry date is left untouched. Sorting entries
/// whose dates cannot be read would be guessing at their order, and `check`
/// reports the unreadable heading instead.
fn reorder_entries(lines: &mut Vec<String>, repairs: &mut Repairs) {
    let (head, blocks) = split_entries(lines);
    if blocks.len() < 2 {
        return;
    }
    let mut dates = Vec::with_capacity(blocks.len());
    for block in &blocks {
        match entry_date(&block[0]) {
            Some(date) => dates.push(date.to_string()),
            None => return,
        }
    }

    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by(|a, b| dates[*b].cmp(&dates[*a]));
    let moved = order
        .iter()
        .enumerate()
        .filter(|(position, from)| position != *from)
        .count();
    if moved == 0 {
        return;
    }
    repairs.reordered_entries += moved;

    // A block carries the blank lines that followed its entry, and the block
    // that was last in the file has none — it ended at end of file. Moving it
    // anywhere but last would butt its final bullet against the next heading,
    // so every block is normalized to one trailing blank and the file's own
    // trailing blanks are trimmed back off at the end. Reassembling without
    // this introduced a blanks-around-headings violation in the most ordinary
    // case there is: two entries, in the wrong order, nothing else wrong.
    let mut out = head;
    for from in order {
        let mut block = blocks[from].clone();
        while block.last().is_some_and(|line| line.trim().is_empty()) {
            block.pop();
        }
        block.push(String::new());
        out.extend(block);
    }
    while out.last().is_some_and(|line| line.trim().is_empty()) {
        out.pop();
    }
    *lines = out;
}

/// Add every required section the entries never had.
fn backfill_sections(lines: &mut Vec<String>, repairs: &mut Repairs) {
    let (head, blocks) = split_entries(lines);
    if blocks.is_empty() {
        return;
    }
    let last = blocks.len() - 1;
    let mut out = head;
    let mut added = 0usize;
    let mut changed_last = false;
    for (position, mut block) in blocks.into_iter().enumerate() {
        let inserted = backfill_entry(&mut block);
        added += inserted;
        changed_last |= inserted > 0 && position == last;
        out.extend(block);
    }
    if added == 0 {
        return;
    }
    // The blank line `backfill_entry` leaves behind separates an entry from
    // the next one. The last entry has no next one, so there that same blank
    // is a trailing blank line at the end of the file — which `MD012` reports,
    // and which would mean this command repaired one lint violation by
    // introducing another.
    if changed_last {
        while out.last().is_some_and(|line| line.trim().is_empty()) {
            out.pop();
        }
    }
    repairs.backfilled_sections += added;
    *lines = out;
}

/// Insert each required section this entry lacks, in template order.
fn backfill_entry(block: &mut Vec<String>) -> usize {
    let mut added = 0usize;
    // The positions are recomputed after each insertion. An entry carries at
    // most three of these and is a few dozen lines long, so tracking the
    // shifted indices by hand would trade clarity for nothing measurable.
    //
    // The bound is not decoration. Recomputing means the loop ends when the
    // inserted heading is *seen*, and an insertion that lands where the mask
    // is not looking is never seen — which is how an entry ending in an
    // unterminated fence turned this into an endless loop that exhausted
    // memory. `first_missing_section` refuses that insertion now, and this
    // bound is what makes termination a property of the loop rather than of
    // that function staying correct.
    while added < REQUIRED_SECTIONS.len()
        && let Some((rank, at)) = first_missing_section(block)
    {
        let mut insertion = vec![
            format!("### {}", SECTIONS[rank]),
            String::new(),
            PLACEHOLDER.to_string(),
            String::new(),
        ];
        // A section appended after the entry's last line of content needs a
        // blank line in front of it. One inserted above an existing section
        // inherits the blank that already separated that section from what
        // came before it.
        if at == 0 || !block[at - 1].trim().is_empty() {
            insertion.insert(0, String::new());
        }
        block.splice(at..at, insertion);
        added += 1;
    }

    if added > 0 {
        // Leave exactly one blank line before the next entry: each insertion
        // carries its own, so an entry that already ended with one would
        // otherwise gain a second.
        while block.last().is_some_and(|line| line.trim().is_empty()) {
            block.pop();
        }
        block.push(String::new());
    }
    added
}

/// The first required section this entry lacks, and where it belongs.
///
/// "First" is template order, and the position is just above the earliest
/// section that should follow it — or past the entry's last line of content
/// when nothing should.
///
/// Both halves read the structural mask. A `### Evidence` quoted inside a fence
/// is not a section this entry has, and the last line of a fenced block is not
/// somewhere a section may be inserted. Getting only the first of those right
/// would suppress a backfill the entry needs; getting only the second right
/// would put the backfill in the wrong place. Getting neither right is what
/// this function did, and it wrote a generated section into the middle of an
/// author's code block.
fn first_missing_section(block: &[String]) -> Option<(usize, usize)> {
    // An entry with an unterminated fence has no knowable end: everything from
    // that fence onward is an example, so there is nowhere in it a section may
    // be placed. This is the same answer the unreadable-date case gives —
    // leave it alone and let `check` report it — and refusing here is what
    // stops the caller inserting a heading the next pass cannot see, forever.
    if crate::model::has_unterminated_fence(block.iter().map(String::as_str)) {
        return None;
    }
    let structural_lines = structural(block);
    let present: Vec<(usize, usize)> = block
        .iter()
        .enumerate()
        .filter(|(at, _)| structural_lines[*at])
        .filter_map(|(at, line)| {
            let label = line.strip_prefix("### ")?.trim();
            let rank = SECTIONS.iter().position(|section| *section == label)?;
            Some((rank, at))
        })
        .collect();

    let rank = REQUIRED_SECTIONS.iter().find_map(|required| {
        let rank = SECTIONS
            .iter()
            .position(|section| section == required)
            .expect("a required section is one the renderer knows");
        (!present.iter().any(|(present, _)| *present == rank)).then_some(rank)
    })?;

    // Past the entry's last line, fenced or not: a section appended after a
    // code block goes after the whole block, never between its last line and
    // its closing delimiter.
    let tail = block
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(block.len(), |at| at + 1);
    let at = present
        .iter()
        .filter(|(other, _)| *other > rank)
        .map(|(_, at)| *at)
        .min()
        .unwrap_or(tail);

    Some((rank, at))
}
