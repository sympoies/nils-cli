//! `issue tracker show` atom.
//!
//! Spec / ops: `cli.forge-cli.issue.tracker.show.v1`. Reads a program tracker
//! issue once and serializes what a board needs: the tracker's `title`,
//! `state`, `url` and labels, and the phase-table rows with their lane
//! references, done flags, phases, dependencies, and notes.
//!
//! Row findings never fail the command. `lint` and `graph` refuse a table with
//! findings, but a board still shows the lanes of an imperfect tracker, so the
//! valid rows are always listed and the findings ride along.

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, IssueTrackerShowArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::issue_tracker::{TrackerFinding, TrackerTarget, read_issue, render_findings};
use crate::ops::issue_view;
use crate::provider::{ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;
use crate::tracker::{self, Row};

const SCHEMA: &str = "issue.tracker.show";
const SCHEMA_VERSION: u32 = 1;

/// The tracker a `show` reads: an issue number and, when the ref names one,
/// the repository that replaces `--repo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowTarget {
    pub repo: Option<String>,
    pub number: u64,
}

/// Parse `N`, `#N`, or `<repo>#N`. The repository is not validated here:
/// provider detection owns what a repository slug may look like.
pub fn parse_target(text: &str) -> Result<ShowTarget, String> {
    let bad = || format!("expected an issue number, `#N`, or `owner/repo#N`, got `{text}`");
    let (repo, digits) = match text.rsplit_once('#') {
        Some((repo, digits)) => ((!repo.is_empty()).then(|| repo.to_string()), digits),
        None => (None, text),
    };
    if repo
        .as_deref()
        .is_some_and(|repo| repo.contains(char::is_whitespace))
    {
        return Err(bad());
    }
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad());
    }
    let number = digits.parse().map_err(|_| bad())?;
    Ok(ShowTarget { repo, number })
}

/// One lane (or gate) of the phase table.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ShowRow {
    pub id: String,
    pub title: String,
    /// `owner/repo#N`, or `null` for a gate. An own-repository `#N` is
    /// qualified with the tracker's repository when it is known.
    pub reference: Option<String>,
    pub done: bool,
    /// The third-level heading the row sits under.
    pub phase: Option<String>,
    pub after: Vec<String>,
    pub notes: Option<String>,
    /// 1-based line number in the tracker body.
    pub line: usize,
}

/// Envelope payload for `cli.forge-cli.issue.tracker.show.v1`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IssueTrackerShowPayload {
    #[serde(flatten)]
    pub target: TrackerTarget,
    /// The tracker's repository slug, when known.
    pub repo: Option<String>,
    pub title: String,
    /// `open` or `closed`.
    pub state: &'static str,
    pub labels: Vec<String>,
    /// Valid rows; equals `rows.len()`.
    pub row_count: usize,
    pub rows: Vec<ShowRow>,
    /// Row findings; they do not fail the command.
    pub findings: Vec<TrackerFinding>,
}

pub fn run(
    global: &GlobalFlags,
    args: IssueTrackerShowArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let mut global = global.clone();
    if let Some(repo) = args.target.repo {
        global.repo = Some(repo);
    }
    let id = args.target.number;
    if global.is_local() {
        let runner = crate::local::LocalRunner::from_global(&global)?;
        return run_with(&runner, &global, id, format, git_remote_url);
    }
    let runner = default_runner();
    run_with(&runner, &global, id, format, git_remote_url)
}

pub fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    id: u64,
    format: OutputFormat,
    remote_url_lookup: F,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        remote_url_lookup,
    )?;
    if global.dry_run {
        let payload = DryRunPayload::new(ctx.provider, &issue_view::build_view_call(&ctx, id));
        return Ok(emit_success(schema(), payload, format, |p| {
            println!("would run: {plan}", plan = p.plan.join(" "))
        }));
    }
    let payload = compute(runner, &ctx, id)?;
    Ok(emit_success(schema(), payload, format, render_text))
}

/// Read the tracker issue and serialize its rows.
pub fn compute<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    id: u64,
) -> Result<IssueTrackerShowPayload, ForgeError> {
    let view = read_issue(runner, ctx, id)?;
    let table = tracker::parse(&view.body);
    let findings: Vec<TrackerFinding> = tracker::row_findings(&table)
        .iter()
        .map(TrackerFinding::from)
        .collect();
    // A table over the row limit is not analysed, so it lists no rows.
    let too_many = findings
        .iter()
        .any(|finding| finding.code == "too-many-rows");
    let rows: Vec<ShowRow> = if too_many {
        Vec::new()
    } else {
        table
            .rows
            .iter()
            .map(|row| show_row(row, ctx.repo.as_deref()))
            .collect()
    };
    Ok(IssueTrackerShowPayload {
        target: TrackerTarget::issue(&view),
        repo: ctx.repo.clone(),
        title: view.title,
        state: view.state,
        labels: view.labels,
        row_count: rows.len(),
        rows,
        findings,
    })
}

fn show_row(row: &Row, own_repo: Option<&str>) -> ShowRow {
    let reference = row.reference.as_ref().map(|reference| {
        match (&reference.owner, &reference.repo, own_repo) {
            (Some(owner), Some(repo), _) => format!("{owner}/{repo}#{}", reference.number),
            (_, _, Some(own)) => format!("{own}#{}", reference.number),
            _ => reference.to_string(),
        }
    });
    ShowRow {
        id: row.id.clone(),
        title: row.title.clone(),
        reference,
        done: row.done,
        phase: row.phase.clone(),
        after: row.after.clone(),
        notes: row.notes.clone(),
        line: row.line,
    }
}

fn schema() -> String {
    schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION)
}

fn render_text(payload: &IssueTrackerShowPayload) {
    println!(
        "{target}: {title} [{state}]",
        target = payload.target.describe(),
        title = payload.title,
        state = payload.state
    );
    for row in &payload.rows {
        let mark = if row.done { 'x' } else { ' ' };
        let reference = row.reference.as_deref().unwrap_or("gate");
        let after = if row.after.is_empty() {
            String::new()
        } else {
            format!(" after {}", row.after.join(", "))
        };
        println!("[{mark}] {id} {reference}{after}", id = row.id);
    }
    render_findings(&payload.findings);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::issue_tracker::testing::{FakeForge, OWN_REPO, ctx};
    use pretty_assertions::assert_eq;

    const TABLE: &str = "## Phase table\n\n### Phase 1\n\n- [x] **A1** First: #11 (PR #12)\n- [ ] **A2** Second: other/repo#3 · after A1\n- [ ] **REL** Release · after A1, A2\n";

    #[test]
    fn parses_the_ref_forms() {
        let target = |repo: Option<&str>, number| ShowTarget {
            repo: repo.map(str::to_string),
            number,
        };
        assert_eq!(parse_target("7"), Ok(target(None, 7)));
        assert_eq!(parse_target("#7"), Ok(target(None, 7)));
        assert_eq!(parse_target("o/r#7"), Ok(target(Some("o/r"), 7)));
        for bad in ["", "#", "o/r#", "x", "o/r#x", "o r#7", "-1", "#7#8x"] {
            assert!(parse_target(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn serializes_rows_with_qualified_references() {
        let forge = FakeForge::with_tracker(TABLE);
        let payload = compute(&forge, &ctx(), 1).expect("show");
        assert_eq!(payload.repo.as_deref(), Some(OWN_REPO));
        assert_eq!(payload.state, "open");
        assert_eq!(payload.row_count, 3);
        let refs: Vec<Option<&str>> = payload
            .rows
            .iter()
            .map(|row| row.reference.as_deref())
            .collect();
        assert_eq!(
            refs,
            [Some("example/tracker#11"), Some("other/repo#3"), None]
        );
        assert_eq!(payload.rows[0].notes.as_deref(), Some("PR #12"));
        assert_eq!(payload.rows[2].after, ["A1", "A2"]);
        assert_eq!(payload.rows[2].phase.as_deref(), Some("Phase 1"));
        assert_eq!(payload.findings, Vec::new());
        assert_eq!(forge.log(), ["view 1"]);
    }

    #[test]
    fn keeps_own_refs_unqualified_without_a_repository() {
        let forge = FakeForge::with_tracker(TABLE);
        let mut ctx = ctx();
        ctx.repo = None;
        let payload = compute(&forge, &ctx, 1).expect("show");
        assert_eq!(payload.rows[0].reference.as_deref(), Some("#11"));
    }

    #[test]
    fn lists_no_rows_over_the_row_limit() {
        let mut body = String::from("## Phase table\n");
        for n in 1..=tracker::MAX_ROWS + 1 {
            body.push_str(&format!("- [ ] **A{n}** Row {n}: #{n}\n"));
        }
        let forge = FakeForge::with_tracker(&body);
        let payload = compute(&forge, &ctx(), 1).expect("show");
        assert_eq!(payload.row_count, 0);
        assert_eq!(payload.rows, Vec::new());
        let codes: Vec<&str> = payload.findings.iter().map(|f| f.code).collect();
        assert_eq!(codes, ["too-many-rows"]);
    }
}
