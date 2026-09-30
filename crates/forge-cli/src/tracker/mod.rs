//! Program tracker phase-table grammar: parse a tracker body into rows, lint
//! it, and generate the dependency graph the rows derive.
//!
//! This module is pure (no IO). The grammar is normative in the
//! `agent-runtime-kit` repository (`tracker-row-grammar.md`); its conformance
//! corpus is vendored under `tests/fixtures/tracker-row-grammar/` and replayed
//! by `tests/integration/tracker_grammar.rs`.
//!
//! The grammar names every character it treats as whitespace: a line ends
//! with spaces, tabs, and carriage returns, and every other trim removes
//! spaces and tabs only. Nothing here may use `str::trim*` without an
//! explicit character set, because those also remove characters the grammar
//! keeps as text (a no-break space, for one).

use std::collections::{HashMap, HashSet};

pub mod edit;

/// Removed from the end of every line.
const LINE_END: [char; 3] = [' ', '\t', '\r'];
/// Removed by every other trim in the grammar.
pub(crate) const BLANK: [char; 2] = [' ', '\t'];

pub(crate) const PHASE_TABLE: &str = "## phase table";
pub(crate) const DEPENDENCY_GRAPH: &str = "## dependency graph";
pub(crate) const OPEN_FENCE: &str = "```mermaid";
pub(crate) const CLOSE_FENCE: &str = "```";

/// Row lines a phase table may hold; a larger table is not analysed. A body is
/// author-controlled text, and 500 rows is the bound the downstream board
/// applies.
pub const MAX_ROWS: usize = 500;
/// Distinct issues `lint --check-state` reads; each one is a provider call
/// that author-controlled text asks for.
pub const MAX_STATE_REFS: usize = 200;

const ROW_MARKS: [&str; 3] = ["- [ ]", "- [x]", "- [X]"];
/// Space, U+00B7 MIDDLE DOT, space, `after`.
const AFTER_MARK: &str = " \u{b7} after";

/// A row's issue reference. `owner` and `repo` are both `None` for a `#N`
/// ref, which names the tracker's own repository.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IssueRef {
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub number: u64,
}

impl std::fmt::Display for IssueRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let (Some(owner), Some(repo)) = (&self.owner, &self.repo) {
            write!(f, "{owner}/{repo}")?;
        }
        write!(f, "#{}", self.number)
    }
}

/// One phase-table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub title: String,
    /// `None` for a gate (a row without a ref).
    pub reference: Option<IssueRef>,
    pub notes: Option<String>,
    pub after: Vec<String>,
    pub done: bool,
    pub phase: Option<String>,
    /// 1-based line number in the body.
    pub line: usize,
}

/// Grammar finding codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingCode {
    MalformedRow,
    DuplicateId,
    UnknownDependency,
    SelfDependency,
    Cycle,
    StaleGraph,
    /// Not a grammar code: the table has more than [`MAX_ROWS`] row lines and
    /// was not analysed.
    TooManyRows,
}

impl FindingCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MalformedRow => "malformed-row",
            Self::DuplicateId => "duplicate-id",
            Self::UnknownDependency => "unknown-dependency",
            Self::SelfDependency => "self-dependency",
            Self::Cycle => "cycle",
            Self::StaleGraph => "stale-graph",
            Self::TooManyRows => "too-many-rows",
        }
    }
}

/// One grammar finding. `line` and `ids` follow the corpus: see the table in
/// the fixture README.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: FindingCode,
    pub line: Option<usize>,
    pub ids: Vec<String>,
}

impl Finding {
    fn new(code: FindingCode, line: Option<usize>, ids: Vec<String>) -> Self {
        Self { code, line, ids }
    }

    /// One-line explanation for a reader; not part of the grammar.
    pub fn message(&self) -> String {
        let id = |at: usize| self.ids.get(at).map(String::as_str).unwrap_or("?");
        match self.code {
            FindingCode::MalformedRow => "row does not match the tracker row grammar".to_string(),
            FindingCode::DuplicateId => format!("id {} is already used by an earlier row", id(0)),
            FindingCode::UnknownDependency => format!(
                "{} depends on {}, which is not the id of any row",
                id(0),
                id(1)
            ),
            FindingCode::SelfDependency => format!("{} lists itself in its after clause", id(0)),
            FindingCode::Cycle => format!("dependency cycle: {}", self.ids.join(", ")),
            FindingCode::StaleGraph => {
                "the mermaid block in the Dependency graph section is missing or not current"
                    .to_string()
            }
            FindingCode::TooManyRows => {
                format!("the phase table has more than {MAX_ROWS} rows and was not analysed")
            }
        }
    }
}

/// The phase table of a body: its valid rows in table order plus the line
/// numbers of the row lines that do not match the grammar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub rows: Vec<Row>,
    pub malformed: Vec<usize>,
}

/// Rows and every grammar finding for one body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub rows: Vec<Row>,
    pub findings: Vec<Finding>,
}

/// A line with its line end removed.
pub(crate) fn line_text(raw: &str) -> &str {
    raw.trim_end_matches(LINE_END)
}

/// The body as grammar lines: split on line feeds, line ends removed.
pub(crate) fn lines(body: &str) -> Vec<&str> {
    body.split('\n').map(line_text).collect()
}

fn trim(text: &str) -> &str {
    text.trim_matches(BLANK)
}

/// Line indexes of a `## <heading>` section: its heading line and the line
/// after its last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Section {
    pub heading: usize,
    pub end: usize,
}

/// The first section whose heading equals `heading` without regard to ASCII
/// case. It ends before the next line that starts with `## `.
pub(crate) fn section(lines: &[&str], heading: &str) -> Option<Section> {
    let start = lines
        .iter()
        .position(|line| line.eq_ignore_ascii_case(heading))?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.starts_with("## "))
        .map_or(lines.len(), |offset| start + 1 + offset);
    Some(Section {
        heading: start,
        end,
    })
}

/// Where the dependency graph block is, by line index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphLocation {
    /// The body has no `## Dependency graph` section.
    NoSection,
    /// The section has no opening fence, or its block is never closed.
    NoBlock { heading: usize },
    /// The opening and closing fence lines of the block.
    Block { open: usize, close: usize },
}

pub(crate) fn locate_graph(lines: &[&str]) -> GraphLocation {
    let Some(section) = section(lines, DEPENDENCY_GRAPH) else {
        return GraphLocation::NoSection;
    };
    let body = section.heading + 1..section.end;
    let block = body
        .clone()
        .find(|&at| lines[at] == OPEN_FENCE)
        .and_then(|open| {
            (open + 1..body.end)
                .find(|&at| lines[at] == CLOSE_FENCE)
                .map(|close| (open, close))
        });
    match block {
        Some((open, close)) => GraphLocation::Block { open, close },
        None => GraphLocation::NoBlock {
            heading: section.heading,
        },
    }
}

/// An id: an upper-case ASCII letter followed by ASCII letters or digits.
fn is_id(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|ch| ch.is_ascii_alphanumeric())
}

/// An owner or repository name in a ref.
fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// `#N` or `owner/repo#N`, where `N` is one to fifteen digits and does not
/// start with `0`.
fn parse_ref(text: &str) -> Option<IssueRef> {
    let (repository, digits) = text.rsplit_once('#')?;
    if digits.is_empty()
        || digits.len() > 15
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let number = digits.parse().ok()?;
    if repository.is_empty() {
        return Some(IssueRef {
            owner: None,
            repo: None,
            number,
        });
    }
    let (owner, repo) = repository.split_once('/')?;
    (is_name(owner) && is_name(repo)).then(|| IssueRef {
        owner: Some(owner.to_string()),
        repo: Some(repo.to_string()),
        number,
    })
}

/// Byte offsets inside a row line that [`edit::tick`] writes at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowSpans {
    /// End of the text before the ` · after` clause, trimmed: where a notes
    /// group goes when the row has none.
    pub text_end: usize,
    /// The opening and closing parenthesis of the notes group.
    pub notes: Option<(usize, usize)>,
}

/// A row line of the phase table, by line index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowLine<'a> {
    Row(Box<Row>, RowSpans),
    /// The line does not match the grammar. `shown_id` is the id its
    /// `- [<state>] **<id>**` prefix shows, when it has one.
    Malformed {
        shown_id: Option<&'a str>,
    },
}

/// Split `- [<state>] **<id>**` off a row line: the done state, the id, and
/// the byte offset of what follows.
fn row_prefix(line: &str) -> Option<(bool, &str, usize)> {
    let done = match line.get(..6)? {
        "- [ ] " => false,
        "- [x] " | "- [X] " => true,
        _ => return None,
    };
    let rest = line[6..].strip_prefix("**")?;
    let id = &rest[..rest.find("**")?];
    is_id(id).then_some((done, id, 6 + 2 + id.len() + 2))
}

/// Parse one row line (line end already removed). `None` means malformed.
fn parse_row(line: &str) -> Option<(Row, RowSpans)> {
    let (done, id, tail_start) = row_prefix(line)?;
    let tail = &line[tail_start..];
    if !tail.starts_with(' ') {
        return None;
    }

    // 1. Dependencies: the last marker that a space or the line end follows.
    let clause = tail
        .rmatch_indices(AFTER_MARK)
        .map(|(at, _)| at)
        .find(|at| {
            let next = &tail[at + AFTER_MARK.len()..];
            next.is_empty() || next.starts_with(' ')
        });
    let mut after: Vec<String> = Vec::new();
    if let Some(at) = clause {
        let mut seen: HashSet<&str> = HashSet::new();
        for entry in trim(&tail[at + AFTER_MARK.len()..]).split(',') {
            let entry = trim(entry);
            if !is_id(entry) || !seen.insert(entry) {
                return None;
            }
            after.push(entry.to_string());
        }
    }
    let head = clause.map_or(tail, |at| &tail[..at]);
    let text_start = tail_start + (head.len() - head.trim_start_matches(BLANK).len());
    let mut text = trim(head);
    let text_end = text_start + text.len();

    // 2. Notes: a parenthesised group that ends the remaining text.
    let mut notes = None;
    let mut notes_span = None;
    if text.ends_with(')') {
        let mut depth = 0usize;
        let mut opening = None;
        for (at, ch) in text.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                opening = Some(at);
                break;
            }
        }
        if let Some(open) = opening {
            let inner = trim(&text[open + 1..text.len() - 1]);
            if open > 0 && text.as_bytes()[open - 1] == b' ' && !inner.is_empty() {
                notes = Some(inner.to_string());
                notes_span = Some((text_start + open, text_end - 1));
                text = trim(&text[..open]);
            }
        }
    }

    // 3. Ref: `: <ref>` at the end of what remains.
    let mut reference = None;
    if let Some((head, candidate)) = text.rsplit_once(' ')
        && let Some(head) = head.strip_suffix(':')
        && let Some(parsed) = parse_ref(candidate)
    {
        reference = Some(parsed);
        text = trim(head);
    }
    if text.is_empty() {
        return None;
    }

    Some((
        Row {
            id: id.to_string(),
            title: text.to_string(),
            reference,
            notes,
            after,
            done,
            phase: None,
            line: 0,
        },
        RowSpans {
            text_end,
            notes: notes_span,
        },
    ))
}

/// Every row line of the phase table with its line index, in table order.
pub(crate) fn scan<'a>(lines: &[&'a str]) -> Vec<(usize, RowLine<'a>)> {
    let Some(table) = section(lines, PHASE_TABLE) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut phase: Option<String> = None;
    let body = lines.iter().enumerate().take(table.end);
    for (at, &line) in body.skip(table.heading + 1) {
        if let Some(name) = line.strip_prefix("### ") {
            let name = trim(name);
            if !name.is_empty() {
                phase = Some(name.to_string());
            }
        } else if ROW_MARKS.iter().any(|mark| line.starts_with(mark)) {
            let row_line = match parse_row(line) {
                Some((mut row, spans)) => {
                    row.phase = phase.clone();
                    row.line = at + 1;
                    RowLine::Row(Box::new(row), spans)
                }
                None => RowLine::Malformed {
                    shown_id: row_prefix(line).map(|(_, id, _)| id),
                },
            };
            found.push((at, row_line));
        }
    }
    found
}

/// Whether `body` has a `## Phase table` section.
pub fn has_phase_table(body: &str) -> bool {
    section(&lines(body), PHASE_TABLE).is_some()
}

/// Read the phase table of `body`. A body without one has no rows.
pub fn parse(body: &str) -> Table {
    let mut table = Table::default();
    for (at, row_line) in scan(&lines(body)) {
        match row_line {
            RowLine::Row(row, _) => table.rows.push(*row),
            RowLine::Malformed { .. } => table.malformed.push(at + 1),
        }
    }
    table
}

/// The row findings of a table: every code except `stale-graph`. A table
/// with more than [`MAX_ROWS`] row lines is not analysed and reports only
/// `too-many-rows`.
pub fn row_findings(table: &Table) -> Vec<Finding> {
    if table.rows.len() + table.malformed.len() > MAX_ROWS {
        return vec![Finding::new(FindingCode::TooManyRows, None, Vec::new())];
    }
    let mut findings: Vec<Finding> = table
        .malformed
        .iter()
        .map(|&line| Finding::new(FindingCode::MalformedRow, Some(line), Vec::new()))
        .collect();

    // Distinct ids in table order, each with the line of its first row. A row
    // that reuses an id still counts for the dependency findings below.
    let mut ids: Vec<(&str, usize)> = Vec::new();
    let mut index_of: HashMap<&str, usize> = HashMap::new();
    for row in &table.rows {
        if index_of.contains_key(row.id.as_str()) {
            findings.push(Finding::new(
                FindingCode::DuplicateId,
                Some(row.line),
                vec![row.id.clone()],
            ));
        } else {
            index_of.insert(&row.id, ids.len());
            ids.push((&row.id, row.line));
        }
    }
    findings.sort_by_key(|finding| finding.line);

    // dependents[p] holds every id that depends on p.
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
    for row in &table.rows {
        for dependency in &row.after {
            if *dependency == row.id {
                findings.push(Finding::new(
                    FindingCode::SelfDependency,
                    Some(row.line),
                    vec![row.id.clone()],
                ));
            } else if let Some(&prerequisite) = index_of.get(dependency.as_str()) {
                dependents[prerequisite].push(index_of[row.id.as_str()]);
            } else {
                findings.push(Finding::new(
                    FindingCode::UnknownDependency,
                    Some(row.line),
                    vec![row.id.clone(), dependency.clone()],
                ));
            }
        }
    }

    // A cycle is a largest set of ids that all reach one another.
    let reach: Vec<Vec<bool>> = (0..ids.len())
        .map(|start| {
            let mut seen = vec![false; ids.len()];
            let mut stack = dependents[start].clone();
            while let Some(node) = stack.pop() {
                if !seen[node] {
                    seen[node] = true;
                    stack.extend(&dependents[node]);
                }
            }
            seen
        })
        .collect();
    let mut grouped = vec![false; ids.len()];
    for first in 0..ids.len() {
        if grouped[first] {
            continue;
        }
        let group: Vec<usize> = (0..ids.len())
            .filter(|&other| other == first || (reach[first][other] && reach[other][first]))
            .collect();
        if group.len() > 1 {
            findings.push(Finding::new(
                FindingCode::Cycle,
                Some(ids[first].1),
                group
                    .iter()
                    .map(|&member| ids[member].0.to_string())
                    .collect(),
            ));
            for member in group {
                grouped[member] = true;
            }
        }
    }
    findings
}

/// The canonical Mermaid lines for `rows`: `graph LR`, one node line per row
/// in table order, then one edge line per dependency.
pub fn generate(rows: &[Row]) -> Vec<String> {
    let mut graph = vec!["graph LR".to_string()];
    for row in rows {
        graph.push(match row.reference {
            Some(_) => format!("  {}", row.id),
            None => format!("  {id}{{{{{id}}}}}", id = row.id),
        });
    }
    for row in rows {
        for dependency in &row.after {
            graph.push(format!("  {dependency} --> {}", row.id));
        }
    }
    graph
}

/// The lines of the body's dependency graph block, or `None` when the block
/// is missing.
pub fn graph_block(body: &str) -> Option<Vec<String>> {
    let lines = lines(body);
    match locate_graph(&lines) {
        GraphLocation::Block { open, close } => Some(
            lines[open + 1..close]
                .iter()
                .map(|line| line.to_string())
                .collect(),
        ),
        _ => None,
    }
}

/// Parse `body` and report every grammar finding. `stale-graph` is reported
/// only when the table has no row finding, because only then is there a
/// generated graph to compare the block with.
pub fn lint(body: &str) -> Report {
    let table = parse(body);
    let mut findings = row_findings(&table);
    if findings.is_empty() && graph_block(body) != Some(generate(&table.rows)) {
        findings.push(Finding::new(FindingCode::StaleGraph, None, Vec::new()));
    }
    Report {
        rows: table.rows,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const LF: &str = "Intro.\n\n## Phase table\n\n### Phase 1\n\n- [ ] **A1** First: #1\n- [x] **A2** Second: example/alpha#2 (PR #3) · after A1\n- [ ] **REL** Release · after A2\n\n## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL\n```\n";

    fn ids(report: &Report) -> Vec<&str> {
        report.rows.iter().map(|row| row.id.as_str()).collect()
    }

    #[test]
    fn baseline_body_is_clean() {
        let report = lint(LF);
        assert_eq!(report.findings, Vec::new());
        assert_eq!(ids(&report), ["A1", "A2", "REL"]);
        assert_eq!(report.rows[1].notes.as_deref(), Some("PR #3"));
        assert_eq!(report.rows[1].line, 8);
        assert_eq!(report.rows[2].phase.as_deref(), Some("Phase 1"));
    }

    #[test]
    fn carriage_returns_end_lines() {
        let crlf = LF.replace('\n', "\r\n");
        assert_eq!(lint(&crlf), lint(LF));
        assert_eq!(graph_block(&crlf), Some(generate(&lint(LF).rows)));
    }

    #[test]
    fn trailing_spaces_and_tabs_end_lines() {
        let padded: String = LF
            .split('\n')
            .map(|line| format!("{line} \t \t"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(lint(&padded), lint(LF));
        assert_eq!(ids(&lint(&padded)), ["A1", "A2", "REL"]);
        let mixed: String = LF
            .split('\n')
            .map(|line| format!("{line}\t \r"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(lint(&mixed), lint(LF));
    }

    #[test]
    fn a_carriage_return_inside_a_line_is_text() {
        let report = lint("## Phase table\n- [ ] **A1** Fir\rst: #1\n");
        assert_eq!(report.rows[0].title, "Fir\rst");
    }

    #[test]
    fn only_space_tab_and_line_end_cr_are_whitespace() {
        // Unicode whitespace that `str::trim` would remove is ordinary text:
        // an em space, a vertical tab, a form feed, and a no-break space.
        for ch in ['\u{2003}', '\u{b}', '\u{c}', '\u{a0}'] {
            let body = format!("## Phase table\n### {ch}Name{ch}\n- [ ] **A1** Title{ch}\n");
            let report = lint(&body);
            assert_eq!(report.rows.len(), 1, "{ch:?}");
            assert_eq!(report.rows[0].title, format!("Title{ch}"), "{ch:?}");
            assert_eq!(
                report.rows[0].phase.as_deref(),
                Some(format!("{ch}Name{ch}").as_str()),
                "{ch:?}"
            );
            // A heading followed by such a character is another heading.
            let other = format!("## Phase table{ch}\n- [ ] **A1** Title\n");
            assert_eq!(lint(&other).rows, Vec::new(), "{ch:?}");
        }
    }

    #[test]
    fn a_trailing_blank_makes_a_bare_id_row_malformed() {
        // The line end is removed first, so nothing follows the id.
        let report = lint("## Phase table\n- [ ] **A1** \t\r\n");
        assert_eq!(report.rows, Vec::new());
        assert_eq!(
            report.findings,
            vec![Finding {
                code: FindingCode::MalformedRow,
                line: Some(2),
                ids: Vec::new(),
            }]
        );
    }

    #[test]
    fn a_table_with_row_findings_reports_no_stale_graph() {
        let report = lint("## Phase table\n- [ ] **A1** First: #1 · after A9\n");
        assert_eq!(
            report.findings,
            vec![Finding {
                code: FindingCode::UnknownDependency,
                line: Some(2),
                ids: vec!["A1".into(), "A9".into()],
            }]
        );
        assert_eq!(row_findings(&parse("## Phase table\n")), Vec::new());
    }

    #[test]
    fn an_id_starts_with_an_upper_case_ascii_letter() {
        let report = lint(
            "## Phase table\n- [ ] **a1** Lower-case id: #1\n- [ ] **A1** First: #2\n- [ ] **Ab2** Second: #3 · after A1\n- [ ] **A3** Lower-case dependency: #4 · after a1\n- [ ] **É1** Not ASCII: #5\n",
        );
        assert_eq!(ids(&report), ["A1", "Ab2"]);
        let malformed: Vec<Option<usize>> = report.findings.iter().map(|f| f.line).collect();
        assert_eq!(malformed, [Some(2), Some(5), Some(6)]);
        assert!(
            report
                .findings
                .iter()
                .all(|f| f.code == FindingCode::MalformedRow)
        );
    }

    #[test]
    fn one_row_with_a_very_long_after_list_parses_in_linear_time() {
        // A body is author-controlled text: one row may carry tens of thousands
        // of `after` entries, and the repeat check must not be quadratic in them.
        let entries: Vec<String> = (0..60_000).map(|n| format!("B{n}")).collect();
        let body = format!(
            "## Phase table\n- [ ] **A1** Wide: #1 · after {}\n",
            entries.join(", ")
        );
        let started = std::time::Instant::now();
        let report = lint(&body);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "parsing took {:?}",
            started.elapsed()
        );
        assert_eq!(ids(&report), ["A1"]);
        assert_eq!(report.rows[0].after.len(), 60_000);
    }

    fn table_of(rows: usize) -> String {
        let mut body = String::from("## Phase table\n");
        for n in 1..=rows {
            body.push_str(&format!("- [ ] **A{n}** Row {n}: #{n}\n"));
        }
        body
    }

    #[test]
    fn a_table_over_the_row_limit_is_not_analysed() {
        let too_many = vec![Finding {
            code: FindingCode::TooManyRows,
            line: None,
            ids: Vec::new(),
        }];

        // At the limit the table is analysed as usual.
        let at_limit = lint(&table_of(MAX_ROWS));
        assert_eq!(at_limit.rows.len(), MAX_ROWS);
        assert_eq!(
            at_limit.findings,
            vec![Finding {
                code: FindingCode::StaleGraph,
                line: None,
                ids: Vec::new(),
            }]
        );

        // One more row line and only the limit is reported, whatever else the
        // table would have shown.
        let over = format!(
            "{}- [ ] **A1** Reuses an id and names a missing one: #1 · after Z9\n",
            table_of(MAX_ROWS)
        );
        assert_eq!(lint(&over).findings, too_many);
        assert_eq!(row_findings(&parse(&over)), too_many);

        // A malformed row line counts as a row line too.
        let malformed = format!("{}- [ ] not a row\n", table_of(MAX_ROWS));
        assert_eq!(lint(&malformed).findings, too_many);
        assert!(lint(&over).findings[0].message().contains("500"));
    }

    #[test]
    fn has_phase_table_finds_only_the_exact_section() {
        assert!(has_phase_table("Intro\n## PHASE table \r\n"));
        assert!(has_phase_table("## Phase table\n"));
        assert!(!has_phase_table(
            "## Phase table notes\n- [ ] **A1** x: #1\n"
        ));
        assert!(!has_phase_table("No sections.\n"));
        assert!(!has_phase_table(""));
    }

    #[test]
    fn generate_orders_nodes_then_edges() {
        assert_eq!(generate(&[]), ["graph LR"]);
        assert_eq!(
            generate(&lint(LF).rows),
            [
                "graph LR",
                "  A1",
                "  A2",
                "  REL{{REL}}",
                "  A1 --> A2",
                "  A2 --> REL"
            ]
        );
    }
}
