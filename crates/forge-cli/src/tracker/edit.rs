//! Targeted tracker body transformations: rewrite the dependency graph block
//! or one phase-table row and leave every other byte of the body untouched.
//!
//! A body is handled as the segments between line feeds, so untouched lines
//! keep their exact bytes, trailing blanks and carriage returns included.
//! Inserted lines follow the line ending in use where they land.

use super::{
    BLANK, CLOSE_FENCE, GraphLocation, MAX_ROWS, OPEN_FENCE, PHASE_TABLE, RowLine, line_text,
    locate_graph, scan, section,
};

const GRAPH_HEADING: &str = "## Dependency graph";

/// What [`write_graph`] did to the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphChange {
    /// The block already holds the generated lines.
    None,
    /// The lines inside the existing block were replaced.
    ReplacedBlock,
    /// The section had no block; one was inserted right after its heading.
    InsertedBlock,
    /// The body had no section; one was inserted right after the phase table.
    InsertedSection,
}

impl GraphChange {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ReplacedBlock => "replaced-block",
            Self::InsertedBlock => "inserted-block",
            Self::InsertedSection => "inserted-section",
        }
    }
}

/// Result of [`write_graph`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdit {
    pub body: String,
    pub change: GraphChange,
}

/// Why [`tick`] refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickError {
    /// No row carries the item id.
    UnknownItem,
    /// More than one valid row carries the item id.
    DuplicateItem { lines: Vec<usize> },
    /// A row line showing the item id does not match the grammar.
    MalformedRow { lines: Vec<usize> },
    /// The PR reference cannot be recorded in a notes group.
    InvalidPr,
    /// The phase table has more than [`MAX_ROWS`] row lines.
    TooManyRows,
}

/// Result of [`tick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickEdit {
    pub body: String,
    /// 1-based line number of the row.
    pub line: usize,
    /// The row line before and after the edit, line end removed.
    pub before: String,
    pub after: String,
}

impl TickEdit {
    pub fn changed(&self) -> bool {
        self.before != self.after
    }
}

/// Whether the line ending in use just before segment `at` is CR LF. Only a
/// segment that a line feed follows can tell; the last segment cannot.
fn uses_crlf(raw: &[&str], at: usize) -> bool {
    let terminated = raw.len() - 1;
    let probe = at.saturating_sub(1).min(terminated.saturating_sub(1));
    terminated > 0 && raw[probe].ends_with('\r')
}

/// Replace `remove` segments at `at` with `new` lines and rejoin the body.
fn splice(raw: &[&str], at: usize, remove: usize, new: &[&str]) -> String {
    let crlf = uses_crlf(raw, at);
    let appending = at == raw.len();
    let mut out: Vec<String> = raw[..at].iter().map(|line| line.to_string()).collect();
    if crlf
        && appending
        && let Some(last) = out.last_mut()
        && !last.ends_with('\r')
    {
        // The old last line is no longer last, so it gets a full line ending.
        last.push('\r');
    }
    for (index, line) in new.iter().enumerate() {
        let ends_body = appending && index + 1 == new.len();
        out.push(if crlf && !ends_body {
            format!("{line}\r")
        } else {
            line.to_string()
        });
    }
    out.extend(raw[at + remove..].iter().map(|line| line.to_string()));
    out.join("\n")
}

/// Put `graph` into the `mermaid` block of the `## Dependency graph` section.
///
/// - An existing block: only the lines between its fences change.
/// - A section without a block: the block goes right after the heading.
/// - No section: the section goes right after the phase table section, or at
///   the end of a body that has no phase table. (`issue tracker graph --write`
///   refuses a body without a phase table before it gets here.)
///
/// A block that already holds `graph` leaves the body as it is.
pub fn write_graph(body: &str, graph: &[String]) -> GraphEdit {
    let raw: Vec<&str> = body.split('\n').collect();
    let lines: Vec<&str> = raw.iter().map(|line| line_text(line)).collect();
    let graph: Vec<&str> = graph.iter().map(String::as_str).collect();
    let fenced = || {
        let mut block = vec![OPEN_FENCE];
        block.extend(&graph);
        block.push(CLOSE_FENCE);
        block
    };

    let (body, change) = match locate_graph(&lines) {
        GraphLocation::Block { open, close } => {
            if lines[open + 1..close] == graph[..] {
                return GraphEdit {
                    body: body.to_string(),
                    change: GraphChange::None,
                };
            }
            (
                splice(&raw, open + 1, close - open - 1, &graph),
                GraphChange::ReplacedBlock,
            )
        }
        GraphLocation::NoBlock { heading } => {
            let mut new = vec![""];
            new.extend(fenced());
            if lines.get(heading + 1).is_some_and(|next| !next.is_empty()) {
                new.push("");
            }
            (
                splice(&raw, heading + 1, 0, &new),
                GraphChange::InsertedBlock,
            )
        }
        GraphLocation::NoSection => {
            // Before the heading that ends the phase table section; otherwise
            // at the end of the body, ahead of a final line feed.
            let next_heading = section(&lines, PHASE_TABLE)
                .map(|table| table.end)
                .filter(|&end| end < lines.len());
            let at = next_heading.unwrap_or(if raw.last() == Some(&"") {
                raw.len() - 1
            } else {
                raw.len()
            });
            let mut new = Vec::new();
            if at > 0 && !lines[at - 1].is_empty() {
                new.push("");
            }
            new.extend([GRAPH_HEADING, ""]);
            new.extend(fenced());
            if next_heading.is_some() {
                new.push("");
            }
            (splice(&raw, at, 0, &new), GraphChange::InsertedSection)
        }
    };
    GraphEdit { body, change }
}

/// Whether `pr` can be written into a notes group as `PR <pr>` without
/// changing how the row parses: no whitespace, parenthesis, comma, middle
/// dot, or control character.
pub fn pr_is_recordable(pr: &str) -> bool {
    !pr.is_empty()
        && !pr.chars().any(|ch| {
            ch.is_whitespace() || ch.is_control() || matches!(ch, '(' | ')' | ',' | '\u{b7}')
        })
}

/// Whether `notes` already names `entry` (`PR <ref>`) as a whole reference, so
/// `PR #1` is not found inside `PR #16`.
fn names(notes: &str, entry: &str) -> bool {
    let boundary = |ch: Option<char>| !ch.is_some_and(|ch| ch.is_ascii_alphanumeric());
    notes.match_indices(entry).any(|(at, _)| {
        boundary(notes[..at].chars().next_back())
            && boundary(notes[at + entry.len()..].chars().next())
    })
}

/// Tick the row whose id is `item`, and record `pr` in its notes.
///
/// Only that row line changes: its checkbox becomes `[x]`, and `PR <pr>` is
/// added as a new notes group before any ` · after` clause, or appended
/// inside the existing group unless the group already names it. A row that is
/// already ticked with nothing to record comes back unchanged.
pub fn tick(body: &str, item: &str, pr: Option<&str>) -> Result<TickEdit, TickError> {
    if pr.is_some_and(|pr| !pr_is_recordable(pr)) {
        return Err(TickError::InvalidPr);
    }
    let raw: Vec<&str> = body.split('\n').collect();
    let lines: Vec<&str> = raw.iter().map(|line| line_text(line)).collect();

    let row_lines = scan(&lines);
    if row_lines.len() > MAX_ROWS {
        return Err(TickError::TooManyRows);
    }
    let mut rows = Vec::new();
    let mut malformed = Vec::new();
    for (at, row_line) in row_lines {
        match row_line {
            RowLine::Row(row, spans) if row.id == item => rows.push((at, row, spans)),
            RowLine::Malformed { shown_id } if shown_id == Some(item) => malformed.push(at + 1),
            _ => {}
        }
    }
    if !malformed.is_empty() {
        return Err(TickError::MalformedRow { lines: malformed });
    }
    if rows.len() > 1 {
        return Err(TickError::DuplicateItem {
            lines: rows.iter().map(|(at, _, _)| at + 1).collect(),
        });
    }
    let Some((at, row, spans)) = rows.pop() else {
        return Err(TickError::UnknownItem);
    };

    // Notes first: they sit to the right of the checkbox, so its offset holds.
    let before = lines[at];
    let mut after = before.to_string();
    if let Some(pr) = pr {
        let entry = format!("PR {pr}");
        match spans.notes {
            Some((open, close)) => {
                let inner = &before[open + 1..close];
                if !names(inner, &entry) {
                    let end = open + 1 + inner.trim_end_matches(BLANK).len();
                    after.insert_str(end, &format!(", {entry}"));
                }
            }
            None => after.insert_str(spans.text_end, &format!(" ({entry})")),
        }
    }
    if !row.done {
        after.replace_range(3..4, "x");
    }

    let mut edited = raw.clone();
    let line = format!("{after}{}", &raw[at][before.len()..]);
    edited[at] = &line;
    Ok(TickEdit {
        body: edited.join("\n"),
        line: at + 1,
        before: before.to_string(),
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{generate, lint};
    use pretty_assertions::assert_eq;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tracker-row-grammar")
            .join(name);
        String::from_utf8(std::fs::read(path).expect("read fixture")).expect("utf-8 fixture")
    }

    fn graph_of(body: &str) -> Vec<String> {
        generate(&lint(body).rows)
    }

    /// Byte-exact "nothing else changed": `edited` is `original` with the
    /// bytes `removed` at `at` replaced by `inserted`, and nothing more.
    fn assert_spliced(original: &str, edited: &str, at: usize, removed: &str, inserted: &str) {
        assert_eq!(&original[at..at + removed.len()], removed, "splice anchor");
        let expected = format!(
            "{}{inserted}{}",
            &original[..at],
            &original[at + removed.len()..]
        );
        assert_eq!(edited, expected);
    }

    // ----- write_graph -----------------------------------------------------

    #[test]
    fn replaces_only_the_lines_inside_an_existing_block() {
        let body = fixture("invalid/stale-graph--different.md");
        let edit = write_graph(&body, &graph_of(&body));
        assert_eq!(edit.change, GraphChange::ReplacedBlock);
        let stale = "graph LR\n  A1\n  A2\n  A1 --> A2\n";
        let at = body.find(stale).expect("stale block");
        assert_spliced(
            &body,
            &edit.body,
            at,
            stale,
            "graph LR\n  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL\n",
        );
        assert_eq!(lint(&edit.body).findings, Vec::new());
    }

    #[test]
    fn replaces_the_first_block_and_leaves_a_later_block_alone() {
        let body = fixture("invalid/stale-graph--second-block-current.md");
        let edit = write_graph(&body, &graph_of(&body));
        assert_eq!(edit.change, GraphChange::ReplacedBlock);
        let at = body.find("graph LR\n  A1\n```").expect("first block");
        assert_spliced(
            &body,
            &edit.body,
            at,
            "graph LR\n  A1\n",
            "graph LR\n  A1\n  A2\n  A1 --> A2\n",
        );
    }

    #[test]
    fn inserts_the_block_right_after_the_heading_of_a_section_without_one() {
        let body = fixture("invalid/stale-graph--no-block.md");
        let edit = write_graph(&body, &graph_of(&body));
        assert_eq!(edit.change, GraphChange::InsertedBlock);
        let heading = "## Dependency graph\n";
        let at = body.find(heading).expect("heading") + heading.len();
        assert_spliced(
            &body,
            &edit.body,
            at,
            "",
            "\n```mermaid\ngraph LR\n  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL\n```\n",
        );
        assert!(edit.body.ends_with("```\n\n- A1 -> A2\n- A2 -> REL\n"));
        assert_eq!(lint(&edit.body).findings, Vec::new());
    }

    #[test]
    fn separates_an_inserted_block_from_text_that_follows_the_heading_directly() {
        let body = "## Phase table\n- [ ] **A1** First: #1\n## Dependency graph\nProse.\n";
        let edit = write_graph(body, &graph_of(body));
        assert_eq!(edit.change, GraphChange::InsertedBlock);
        assert_eq!(
            edit.body,
            "## Phase table\n- [ ] **A1** First: #1\n## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n```\n\nProse.\n"
        );
    }

    #[test]
    fn inserts_a_block_above_an_unclosed_one_and_keeps_a_missing_final_newline() {
        let body = fixture("invalid/stale-graph--unclosed.md");
        assert!(!body.ends_with('\n'));
        let edit = write_graph(&body, &graph_of(&body));
        assert_eq!(edit.change, GraphChange::InsertedBlock);
        let heading = "## Dependency graph\n";
        let at = body.find(heading).expect("heading") + heading.len();
        assert_spliced(
            &body,
            &edit.body,
            at,
            "",
            "\n```mermaid\ngraph LR\n  A1\n  A2\n  A1 --> A2\n```\n",
        );
        assert!(edit.body.ends_with("  A1 --> A2"));
        assert_eq!(lint(&edit.body).findings, Vec::new());
    }

    #[test]
    fn inserts_the_section_right_after_the_phase_table_section() {
        let body = fixture("invalid/stale-graph--missing-section.md");
        let edit = write_graph(&body, &graph_of(&body));
        assert_eq!(edit.change, GraphChange::InsertedSection);
        let at = body.find("## Open decisions").expect("next section");
        assert_spliced(
            &body,
            &edit.body,
            at,
            "",
            "## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL\n```\n\n",
        );
        assert_eq!(lint(&edit.body).findings, Vec::new());
    }

    #[test]
    fn inserts_the_section_at_the_end_when_the_phase_table_is_last() {
        let with_newline = "## Phase table\n\n- [ ] **A1** First: #1\n";
        let edit = write_graph(with_newline, &graph_of(with_newline));
        assert_eq!(edit.change, GraphChange::InsertedSection);
        assert_eq!(
            edit.body,
            "## Phase table\n\n- [ ] **A1** First: #1\n\n## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n```\n"
        );

        let without_newline = "## Phase table\n\n- [ ] **A1** First: #1";
        let edit = write_graph(without_newline, &graph_of(without_newline));
        assert_eq!(
            edit.body,
            "## Phase table\n\n- [ ] **A1** First: #1\n\n## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n```"
        );

        let blank_tail = "## Phase table\n\n- [ ] **A1** First: #1\n\n";
        let edit = write_graph(blank_tail, &graph_of(blank_tail));
        assert_eq!(
            edit.body,
            "## Phase table\n\n- [ ] **A1** First: #1\n\n## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n```\n"
        );
    }

    #[test]
    fn appends_the_section_to_a_body_without_a_phase_table() {
        let body = fixture("invalid/stale-graph--no-phase-table.md");
        let edit = write_graph(&body, &graph_of(&body));
        assert_eq!(edit.change, GraphChange::InsertedSection);
        assert_spliced(
            &body,
            &edit.body,
            body.len(),
            "",
            "\n## Dependency graph\n\n```mermaid\ngraph LR\n```\n",
        );
        assert_eq!(lint(&edit.body).findings, Vec::new());
    }

    #[test]
    fn a_current_block_leaves_the_body_untouched() {
        for name in [
            "valid/minimal.md",
            "valid/full.md",
            "valid/edge-cases.md",
            "valid/no-rows.md",
        ] {
            let body = fixture(name);
            let edit = write_graph(&body, &graph_of(&body));
            assert_eq!(edit.change, GraphChange::None, "{name}");
            assert_eq!(edit.body, body, "{name}");
        }
    }

    #[test]
    fn every_stale_fixture_becomes_current_and_a_second_write_is_a_no_op() {
        for name in [
            "different",
            "edge-order",
            "gate-shape",
            "indented-close",
            "indented-open",
            "missing-section",
            "no-block",
            "no-phase-table",
            "second-block-current",
            "unclosed",
        ] {
            let body = fixture(&format!("invalid/stale-graph--{name}.md"));
            let first = write_graph(&body, &graph_of(&body));
            assert_ne!(first.change, GraphChange::None, "{name}");
            assert_eq!(lint(&first.body).findings, Vec::new(), "{name}");
            let second = write_graph(&first.body, &graph_of(&first.body));
            assert_eq!(second.change, GraphChange::None, "{name}");
            assert_eq!(second.body, first.body, "{name}");
        }
    }

    #[test]
    fn keeps_carriage_return_line_endings() {
        let lf = fixture("invalid/stale-graph--different.md");
        let crlf = lf.replace('\n', "\r\n");
        let edit = write_graph(&crlf, &graph_of(&crlf));
        assert_eq!(edit.change, GraphChange::ReplacedBlock);
        assert_eq!(
            edit.body,
            write_graph(&lf, &graph_of(&lf)).body.replace('\n', "\r\n")
        );

        for name in ["missing-section", "no-block"] {
            let lf = fixture(&format!("invalid/stale-graph--{name}.md"));
            let crlf = lf.replace('\n', "\r\n");
            let edit = write_graph(&crlf, &graph_of(&crlf));
            assert_eq!(
                edit.body,
                write_graph(&lf, &graph_of(&lf)).body.replace('\n', "\r\n"),
                "{name}"
            );
        }

        // No final line ending: the appended section adds none either.
        let crlf = "## Phase table\r\n\r\n- [ ] **A1** First: #1";
        assert_eq!(
            write_graph(crlf, &graph_of(crlf)).body,
            "## Phase table\r\n\r\n- [ ] **A1** First: #1\r\n\r\n## Dependency graph\r\n\r\n```mermaid\r\ngraph LR\r\n  A1\r\n```"
        );
    }

    // ----- tick ------------------------------------------------------------

    const TABLE: &str = "Intro with - [ ] **S2** lookalike.\n\n## Phase table\n\n### Phase 1\n\n- [ ] **B1** Board leads with trackers: #21\n- [x] **S1** Row grammar: example/alpha#14 (PR example/alpha#16)\n- [ ] **S2** `lint | graph | tick` commands: example/beta#7 · after S1\n- [ ] **REL** Release containing S2 · after S2\n\n## Notes\n\n- [ ] **B1** Not a row, wrong section: #99\n";

    #[test]
    fn tick_changes_only_the_checkbox_of_the_one_row() {
        let edit = tick(TABLE, "B1", None).expect("tick");
        assert!(edit.changed());
        assert_eq!(edit.line, 7);
        assert_eq!(edit.before, "- [ ] **B1** Board leads with trackers: #21");
        assert_eq!(edit.after, "- [x] **B1** Board leads with trackers: #21");
        let at = TABLE.find("- [ ] **B1** Board").expect("row") + 3;
        assert_spliced(TABLE, &edit.body, at, " ", "x");
    }

    #[test]
    fn tick_records_the_pr_before_the_after_clause_when_the_row_has_no_notes() {
        let edit = tick(TABLE, "S2", Some("example/beta#9")).expect("tick");
        assert_eq!(edit.line, 9);
        assert_eq!(
            edit.after,
            "- [x] **S2** `lint | graph | tick` commands: example/beta#7 (PR example/beta#9) · after S1"
        );
        let row = "- [ ] **S2** `lint | graph | tick` commands: example/beta#7 · after S1";
        let at = TABLE.find(row).expect("row");
        assert_spliced(TABLE, &edit.body, at, row, &edit.after);
        let report = lint(&edit.body);
        let s2 = report.rows.iter().find(|row| row.id == "S2").expect("S2");
        assert_eq!(s2.notes.as_deref(), Some("PR example/beta#9"));
        assert_eq!(s2.after, ["S1"]);
        assert!(s2.done);
    }

    #[test]
    fn tick_records_the_pr_on_a_gate_and_on_a_row_without_a_clause() {
        let edit = tick(TABLE, "REL", Some("#30")).expect("tick gate");
        assert_eq!(
            edit.after,
            "- [x] **REL** Release containing S2 (PR #30) · after S2"
        );
        let edit = tick(TABLE, "B1", Some("#22")).expect("tick plain");
        assert_eq!(
            edit.after,
            "- [x] **B1** Board leads with trackers: #21 (PR #22)"
        );
    }

    #[test]
    fn tick_appends_the_pr_inside_existing_notes() {
        let edit = tick(TABLE, "S1", Some("example/alpha#17")).expect("tick");
        assert_eq!(
            edit.after,
            "- [x] **S1** Row grammar: example/alpha#14 (PR example/alpha#16, PR example/alpha#17)"
        );
        let at = TABLE.find("#16)").expect("notes end") + 3;
        assert_spliced(TABLE, &edit.body, at, "", ", PR example/alpha#17");

        let padded = "## Phase table\n- [ ] **N4** Notes are trimmed: #6 (  padded  ) · after N2\n";
        let edit = tick(padded, "N4", Some("#8")).expect("tick padded");
        assert_eq!(
            edit.after,
            "- [x] **N4** Notes are trimmed: #6 (  padded, PR #8  ) · after N2"
        );
    }

    #[test]
    fn tick_does_not_record_the_same_pr_twice() {
        let edit = tick(TABLE, "S1", Some("example/alpha#16")).expect("tick");
        assert!(!edit.changed());
        assert_eq!(edit.body, TABLE);
        assert_eq!(edit.before, edit.after);

        // A longer number is a different PR, in both directions.
        let edit = tick(TABLE, "S1", Some("example/alpha#1")).expect("tick shorter");
        assert_eq!(
            edit.after,
            "- [x] **S1** Row grammar: example/alpha#14 (PR example/alpha#16, PR example/alpha#1)"
        );
        let edit = tick(TABLE, "S1", Some("example/alpha#160")).expect("tick longer");
        assert!(
            edit.after
                .ends_with("(PR example/alpha#16, PR example/alpha#160)")
        );

        // Ticking the open row and recording a PR it already names.
        let named = "## Phase table\n- [ ] **A1** First: #1 (draft, PR #5)\n";
        let edit = tick(named, "A1", Some("#5")).expect("tick named");
        assert_eq!(edit.after, "- [x] **A1** First: #1 (draft, PR #5)");
    }

    #[test]
    fn ticking_a_ticked_row_with_nothing_to_record_is_a_no_op() {
        let edit = tick(TABLE, "S1", None).expect("tick");
        assert!(!edit.changed());
        assert_eq!(edit.body, TABLE);
        assert_eq!(edit.line, 8);

        let upper = "## Phase table\n- [X] **A1** First: #1\n";
        let edit = tick(upper, "A1", None).expect("tick upper");
        assert!(!edit.changed());
        assert_eq!(edit.body, upper);
    }

    #[test]
    fn tick_keeps_line_endings_and_trailing_blanks() {
        let crlf =
            "## Phase table\r\n- [ ] **A1** First: #1 · after A2 \t\r\n- [ ] **A2** Second: #2\r\n";
        let edit = tick(crlf, "A1", Some("#3")).expect("tick");
        assert_eq!(
            edit.body,
            "## Phase table\r\n- [x] **A1** First: #1 (PR #3) · after A2 \t\r\n- [ ] **A2** Second: #2\r\n"
        );
        assert_eq!(edit.after, "- [x] **A1** First: #1 (PR #3) · after A2");
    }

    #[test]
    fn tick_keeps_a_no_break_space_in_the_title() {
        let body =
            "## Phase table\n- [ ] **W2** A trailing no-break space is title text: #12\u{a0}\n";
        let edit = tick(body, "W2", Some("#13")).expect("tick");
        assert_eq!(
            edit.after,
            "- [x] **W2** A trailing no-break space is title text: #12\u{a0} (PR #13)"
        );
    }

    #[test]
    fn tick_refuses_an_unknown_duplicated_or_malformed_item() {
        assert_eq!(tick(TABLE, "S9", None), Err(TickError::UnknownItem));
        // Ids are case-sensitive, and only the phase table is read.
        assert_eq!(tick(TABLE, "s1", None), Err(TickError::UnknownItem));
        assert_eq!(
            tick("No table.\n- [ ] **A1** x: #1\n", "A1", None),
            Err(TickError::UnknownItem)
        );

        let duplicated = "## Phase table\n- [ ] **A1** First: #1\n- [ ] **A2** Second: #2\n- [ ] **A1** Again: #3\n";
        assert_eq!(
            tick(duplicated, "A1", None),
            Err(TickError::DuplicateItem { lines: vec![2, 4] })
        );
        assert!(tick(duplicated, "A2", None).is_ok());

        let malformed = "## Phase table\n- [ ] **A1** First: #1\n- [ ] **A2** Bad list: #2 · after A1 and more\n";
        assert_eq!(
            tick(malformed, "A2", None),
            Err(TickError::MalformedRow { lines: vec![3] })
        );
        assert!(tick(malformed, "A1", None).is_ok());
    }

    #[test]
    fn tick_refuses_a_table_over_the_row_limit() {
        let mut body = String::from("## Phase table\n");
        for n in 1..=crate::tracker::MAX_ROWS {
            body.push_str(&format!("- [ ] **A{n}** Row {n}: #{n}\n"));
        }
        assert!(tick(&body, "A7", None).is_ok());
        body.push_str("- [ ] not a row\n");
        assert_eq!(tick(&body, "A7", None), Err(TickError::TooManyRows));
    }

    #[test]
    fn tick_refuses_a_pr_reference_that_would_break_the_row() {
        for pr in [
            "",
            " ",
            "a b",
            "#1)",
            "(#1",
            "#1,#2",
            "#1 · after A1",
            "#1\t",
            "a\nb",
        ] {
            assert_eq!(
                tick(TABLE, "B1", Some(pr)),
                Err(TickError::InvalidPr),
                "{pr:?}"
            );
        }
        assert!(tick(TABLE, "B1", Some("group/sub/project!12")).is_ok());
    }
}
