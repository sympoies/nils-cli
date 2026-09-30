//! `issue tracker lint` atom.
//!
//! Spec / ops: `cli.forge-cli.issue.tracker.lint.v1`. Reads a program tracker
//! issue and reports the grammar findings of [`crate::tracker`] plus the
//! findings only a provider can tell: `missing-tracking-label`, and with
//! `--check-state`, `state-mismatch` and `unreadable-ref`. `--body-file` lints
//! a local draft without any provider call, so only the grammar is checked.
//!
//! Any finding exits `DATA 65` with a failure envelope that still carries the
//! payload, so callers read `data.findings[]` either way.

use std::collections::HashMap;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, IssueTrackerLintArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::issue_tracker::{
    TRACKING_LABEL, TrackerFinding, TrackerTarget, emit_findings, missing_target, read_draft,
    read_issue, render_findings,
};
use crate::ops::issue_view;
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;
use crate::tracker::{self, IssueRef, Row};

const SCHEMA: &str = "issue.tracker.lint";
const SCHEMA_VERSION: u32 = 1;

/// Envelope payload for `cli.forge-cli.issue.tracker.lint.v1`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IssueTrackerLintPayload {
    #[serde(flatten)]
    pub target: TrackerTarget,
    /// Valid rows in the phase table.
    pub row_count: usize,
    /// Whether `--check-state` read the referenced issues.
    pub state_checked: bool,
    pub findings: Vec<TrackerFinding>,
}

pub fn run(
    global: &GlobalFlags,
    args: IssueTrackerLintArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let id = match (args.body_file.as_deref(), args.id) {
        (Some(path), _) => return Ok(emit(lint_draft(&read_draft(path)?), format)),
        (None, Some(id)) => id,
        (None, None) => return Err(missing_target()),
    };
    if global.is_local() {
        let runner = crate::local::LocalRunner::from_global(global)?;
        return run_with(
            &runner,
            global,
            id,
            args.check_state,
            format,
            git_remote_url,
        );
    }
    let runner = default_runner();
    run_with(
        &runner,
        global,
        id,
        args.check_state,
        format,
        git_remote_url,
    )
}

pub fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    id: u64,
    check_state: bool,
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
    let payload = compute(runner, global, &ctx, id, check_state)?;
    Ok(emit(payload, format))
}

/// Read the tracker issue and collect every finding.
pub fn compute<R: BackendRunner>(
    runner: &R,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    id: u64,
    check_state: bool,
) -> Result<IssueTrackerLintPayload, ForgeError> {
    let view = read_issue(runner, ctx, id)?;
    let report = tracker::lint(&view.body);
    let mut findings: Vec<TrackerFinding> =
        report.findings.iter().map(TrackerFinding::from).collect();
    if !view.labels.iter().any(|label| label == TRACKING_LABEL) {
        findings.push(TrackerFinding {
            code: "missing-tracking-label",
            line: None,
            ids: Vec::new(),
            message: format!("the issue does not carry the {TRACKING_LABEL} label"),
        });
    }
    if check_state {
        findings.extend(state_findings(runner, global, ctx, &report.rows));
    }
    Ok(IssueTrackerLintPayload {
        target: TrackerTarget::issue(&view),
        row_count: report.rows.len(),
        state_checked: check_state,
        findings,
    })
}

/// Lint a draft body: the grammar only.
pub fn lint_draft(body: &str) -> IssueTrackerLintPayload {
    let report = tracker::lint(body);
    IssueTrackerLintPayload {
        target: TrackerTarget::draft(),
        row_count: report.rows.len(),
        state_checked: false,
        findings: report.findings.iter().map(TrackerFinding::from).collect(),
    }
}

/// What reading one referenced issue gave.
enum RefState {
    Open,
    /// Closed, or merged when the ref names a PR / MR.
    Closed,
    Unreadable(&'static str),
}

/// A referenced issue: the repository to read it from (`None` for the
/// tracker's own) and its number.
type RefKey = (Option<String>, u64);

/// Compare every row that has a ref with its issue's state.
///
/// Each distinct issue is read once through the `issue view` call. An issue
/// named by several rows is delivered in steps, so it is judged as a whole: a
/// closed issue disagrees with each row that is still open, and an open issue
/// disagrees with its rows only once all of them are done. A target that
/// cannot be read is an `unreadable-ref` finding, not an error.
fn state_findings<R: BackendRunner>(
    runner: &R,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    rows: &[Row],
) -> Vec<TrackerFinding> {
    let own_repo = match ctx.provider {
        Provider::Local => Some(crate::local::resolve_slug(global.repo.as_deref())),
        _ => ctx.repo.clone(),
    };
    let key_of = |reference: &IssueRef| -> RefKey {
        let named = match (&reference.owner, &reference.repo) {
            (Some(owner), Some(repo)) => Some(format!("{owner}/{repo}")),
            _ => None,
        };
        let other = named.filter(|named| {
            !own_repo
                .as_deref()
                .is_some_and(|own| own.eq_ignore_ascii_case(named))
        });
        (other, reference.number)
    };

    let mut states: HashMap<RefKey, RefState> = HashMap::new();
    let mut all_done: HashMap<RefKey, bool> = HashMap::new();
    for row in rows {
        let Some(reference) = &row.reference else {
            continue;
        };
        let key = key_of(reference);
        *all_done.entry(key.clone()).or_insert(true) &= row.done;
        states
            .entry(key)
            .or_insert_with_key(|(repo, number)| read_state(runner, ctx, repo.as_deref(), *number));
    }

    let mut findings = Vec::new();
    for row in rows {
        let Some(reference) = &row.reference else {
            continue;
        };
        let key = key_of(reference);
        let (code, message) = match &states[&key] {
            RefState::Unreadable(why) => (
                "unreadable-ref",
                format!("{} cannot read {reference}: {why}", row.id),
            ),
            RefState::Closed if !row.done => (
                "state-mismatch",
                format!("{} is not ticked but {reference} is closed", row.id),
            ),
            RefState::Open if all_done[&key] => (
                "state-mismatch",
                format!("{} is ticked but {reference} is open", row.id),
            ),
            _ => continue,
        };
        findings.push(TrackerFinding {
            code,
            line: Some(row.line),
            ids: vec![row.id.clone()],
            message,
        });
    }
    findings
}

fn read_state<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repo: Option<&str>,
    number: u64,
) -> RefState {
    let target = match repo {
        None => ctx.clone(),
        // One local store holds one repository.
        Some(_) if ctx.provider == Provider::Local => {
            return RefState::Unreadable("the local store holds one repository");
        }
        Some(repo) => ProviderContext {
            repo: Some(repo.to_string()),
            ..ctx.clone()
        },
    };
    match read_issue(runner, &target, number) {
        Ok(view) if view.state == "open" => RefState::Open,
        Ok(_) => RefState::Closed,
        Err(err) => RefState::Unreadable(err.kind()),
    }
}

fn emit(payload: IssueTrackerLintPayload, format: OutputFormat) -> i32 {
    if payload.findings.is_empty() {
        return emit_success(schema(), payload, format, render_text);
    }
    let findings = payload.findings.clone();
    emit_findings(schema(), payload, &findings, format, render_text)
}

fn schema() -> String {
    schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION)
}

fn render_text(payload: &IssueTrackerLintPayload) {
    if payload.findings.is_empty() {
        println!(
            "tracker {target}: {rows} row(s), no findings",
            target = payload.target.describe(),
            rows = payload.row_count,
        );
    } else {
        render_findings(&payload.findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::issue_tracker::TRACKING_LABEL;
    use crate::ops::issue_tracker::testing::{FakeForge, OWN_REPO, ctx, global};
    use crate::tracker::{self, edit::write_graph};
    use pretty_assertions::assert_eq;

    /// `table` followed by its current dependency graph section.
    fn tracker_body(table: &str) -> String {
        let graph = tracker::generate(&tracker::lint(table).rows);
        write_graph(table, &graph).body
    }

    fn summary(
        payload: &IssueTrackerLintPayload,
    ) -> Vec<(&'static str, Option<usize>, Vec<String>)> {
        payload
            .findings
            .iter()
            .map(|f| (f.code, f.line, f.ids.clone()))
            .collect()
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    const TABLE: &str = "## Phase table\n\n- [x] **A1** First: #11\n- [ ] **A2** Second: example/alpha#2 · after A1\n- [ ] **REL** Release · after A2\n";

    #[test]
    fn clean_tracker_has_no_finding_and_reads_only_the_tracker() {
        let forge = FakeForge::with_tracker(&tracker_body(TABLE));
        let payload = compute(&forge, &global(false), &ctx(), 1, false).expect("lint");
        assert_eq!(payload.findings, Vec::new());
        assert_eq!(payload.row_count, 3);
        assert!(!payload.state_checked);
        assert_eq!(payload.target.source, "issue");
        assert_eq!(payload.target.provider, Some("github"));
        assert_eq!(payload.target.number, Some(1));
        assert_eq!(forge.log(), ["view 1"]);
    }

    #[test]
    fn reports_grammar_findings_with_a_message() {
        let body =
            "## Phase table\n\n- [ ] **A1** First: #11 · after A9\n- [ ] **A1** Again: #12\n";
        let forge = FakeForge::with_tracker(body);
        let payload = compute(&forge, &global(false), &ctx(), 1, false).expect("lint");
        assert_eq!(
            summary(&payload),
            [
                ("duplicate-id", Some(4), ids(&["A1"])),
                ("unknown-dependency", Some(3), ids(&["A1", "A9"])),
            ]
        );
        assert!(payload.findings.iter().all(|f| !f.message.is_empty()));
    }

    #[test]
    fn reports_a_stale_graph_once() {
        let forge = FakeForge::with_tracker(TABLE);
        let payload = compute(&forge, &global(false), &ctx(), 1, false).expect("lint");
        assert_eq!(summary(&payload), [("stale-graph", None, Vec::new())]);
    }

    #[test]
    fn reports_a_missing_tracking_label() {
        let forge = FakeForge::default();
        forge.put(
            OWN_REPO,
            1,
            &tracker_body(TABLE),
            "OPEN",
            &["type::feature"],
        );
        let payload = compute(&forge, &global(false), &ctx(), 1, false).expect("lint");
        assert_eq!(
            summary(&payload),
            [("missing-tracking-label", None, Vec::new())]
        );
        assert!(payload.findings[0].message.contains(TRACKING_LABEL));
    }

    #[test]
    fn check_state_reports_each_row_that_disagrees_with_its_issue() {
        let table = "## Phase table\n\n\
            - [x] **D1** Done but open: #11\n\
            - [ ] **O1** Open but closed: #12\n\
            - [ ] **O2** Open but merged: example/alpha#13\n\
            - [x] **D2** Done and closed: #14\n\
            - [ ] **O3** Open and open: #15\n\
            - [x] **U1** Unreadable: example/missing#16\n\
            - [ ] **G1** A gate has no issue to read\n";
        let forge = FakeForge::with_tracker(&tracker_body(table));
        forge.put(OWN_REPO, 11, "", "OPEN", &[]);
        forge.put(OWN_REPO, 12, "", "CLOSED", &[]);
        forge.put("example/alpha", 13, "", "MERGED", &[]);
        forge.put(OWN_REPO, 14, "", "CLOSED", &[]);
        forge.put(OWN_REPO, 15, "", "OPEN", &[]);

        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("lint");
        assert!(payload.state_checked);
        assert_eq!(
            summary(&payload),
            [
                ("state-mismatch", Some(3), ids(&["D1"])),
                ("state-mismatch", Some(4), ids(&["O1"])),
                ("state-mismatch", Some(5), ids(&["O2"])),
                ("unreadable-ref", Some(8), ids(&["U1"])),
            ]
        );
        assert!(payload.findings[0].message.contains("#11"));
        assert!(payload.findings[2].message.contains("example/alpha#13"));

        // `#N` reads the tracker's repository; `owner/repo#N` reads that one.
        let calls = forge.calls.borrow();
        let repo_of = |number: &str| {
            let call = calls
                .iter()
                .find(|argv| argv[2] == "view" && argv[3] == number)
                .unwrap_or_else(|| panic!("no view of {number}"));
            let at = call.iter().position(|arg| arg == "--repo").expect("--repo");
            call[at + 1].clone()
        };
        assert_eq!(repo_of("11"), OWN_REPO);
        assert_eq!(repo_of("13"), "example/alpha");
        assert_eq!(repo_of("16"), "example/missing");
    }

    #[test]
    fn check_state_reads_an_issue_on_two_rows_once_and_judges_it_whole() {
        let table =
            "## Phase table\n\n- [x] **S1** Step one: #20\n- [ ] **S2** Step two: #20 · after S1\n";
        let both_done = table.replace("- [ ] **S2**", "- [x] **S2**");

        // First step delivered, issue still open for the second: no finding.
        let forge = FakeForge::with_tracker(&tracker_body(table));
        forge.put(OWN_REPO, 20, "", "OPEN", &[]);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("lint");
        assert_eq!(payload.findings, Vec::new());
        assert_eq!(forge.log(), ["view 1", "view 20"]);

        // Issue closed while a step is still open: that row disagrees.
        let forge = FakeForge::with_tracker(&tracker_body(table));
        forge.put(OWN_REPO, 20, "", "CLOSED", &[]);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("lint");
        assert_eq!(
            summary(&payload),
            [("state-mismatch", Some(4), ids(&["S2"]))]
        );

        // Every step done but the issue is open: each row disagrees.
        let forge = FakeForge::with_tracker(&tracker_body(&both_done));
        forge.put(OWN_REPO, 20, "", "OPEN", &[]);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("lint");
        assert_eq!(
            summary(&payload),
            [
                ("state-mismatch", Some(3), ids(&["S1"])),
                ("state-mismatch", Some(4), ids(&["S2"])),
            ]
        );
    }

    #[test]
    fn lint_draft_reports_grammar_findings_only() {
        let payload = lint_draft(&tracker_body(TABLE));
        assert_eq!(payload.findings, Vec::new());
        assert_eq!(payload.row_count, 3);
        assert_eq!(payload.target.source, "body-file");
        assert_eq!(payload.target.provider, None);
        assert_eq!(payload.target.number, None);

        let payload = lint_draft(TABLE);
        assert_eq!(summary(&payload), [("stale-graph", None, Vec::new())]);
    }
}
