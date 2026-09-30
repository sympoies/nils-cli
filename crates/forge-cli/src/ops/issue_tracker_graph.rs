//! `issue tracker graph` atom.
//!
//! Spec / ops: `cli.forge-cli.issue.tracker.graph.v1`. Prints the canonical
//! Mermaid block the phase table derives. A table with row findings has no
//! generated graph, so the command refuses with those findings (`DATA 65`).
//!
//! `--write` puts the block into the `## Dependency graph` section through
//! [`crate::tracker::edit::write_graph`], which touches nothing else, and
//! writes only when the block is not already current. The issue is read
//! immediately before the write and the write is a transformation of that
//! read: no body from an earlier read is ever written. `--body-file` does the
//! same on a local draft with no provider call.

use std::fs;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, IssueTrackerGraphArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::issue_tracker::{
    PlannedAction, TrackerFinding, TrackerTarget, emit_findings, missing_target, read_draft,
    read_issue, render_findings, schema_err,
};
use crate::ops::{issue_edit, issue_view};
use crate::provider::{ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;
use crate::tracker::{
    self,
    edit::{GraphChange, write_graph},
};

const SCHEMA: &str = "issue.tracker.graph";
const SCHEMA_VERSION: u32 = 1;

/// Envelope payload for `cli.forge-cli.issue.tracker.graph.v1`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IssueTrackerGraphPayload {
    #[serde(flatten)]
    pub target: TrackerTarget,
    /// The generated Mermaid source without its fence, lines joined by a line
    /// feed. `null` when the table has row findings.
    pub graph: Option<String>,
    /// Whether the body's block already holds the generated lines.
    pub current: bool,
    /// What `--write` does to the body: `none`, `replaced-block`,
    /// `inserted-block`, or `inserted-section`.
    pub change: &'static str,
    /// Whether `--write` changes the body.
    pub changed: bool,
    /// Whether the change was written (never under `--dry-run`).
    pub written: bool,
    pub dry_run: bool,
    /// Row findings that made the command refuse; empty otherwise.
    pub findings: Vec<TrackerFinding>,
    /// Under `--dry-run`, the backend call a real run would make.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PlannedAction>,
}

pub fn run(
    global: &GlobalFlags,
    args: IssueTrackerGraphArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let id = match (args.body_file.as_deref(), args.id) {
        (Some(path), _) => return run_draft(global, path, args.write, format),
        (None, Some(id)) => id,
        (None, None) => return Err(missing_target()),
    };
    if global.is_local() {
        let runner = crate::local::LocalRunner::from_global(global)?;
        return run_with(&runner, global, id, args.write, format, git_remote_url);
    }
    let runner = default_runner();
    run_with(&runner, global, id, args.write, format, git_remote_url)
}

pub fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    id: u64,
    write: bool,
    format: OutputFormat,
    remote_url_lookup: F,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        remote_url_lookup,
    )?;
    // A read-only dry-run plans the one read, like `issue view --dry-run`. With
    // --write the read runs so the planned change can be shown.
    if global.dry_run && !write {
        let payload = DryRunPayload::new(ctx.provider, &issue_view::build_view_call(&ctx, id));
        return Ok(emit_success(schema(), payload, format, |p| {
            println!("would run: {plan}", plan = p.plan.join(" "))
        }));
    }
    let payload = compute(runner, global, &ctx, id, write)?;
    Ok(emit(payload, write, format))
}

/// Read the tracker issue, generate its graph, and with `write` put the block
/// into that freshly read body.
pub fn compute<R: BackendRunner>(
    runner: &R,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    id: u64,
    write: bool,
) -> Result<IssueTrackerGraphPayload, ForgeError> {
    let view = read_issue(runner, ctx, id)?;
    let (mut payload, body) = plan(
        TrackerTarget::issue(&view),
        &view.body,
        write,
        global.dry_run,
    )?;
    if let Some(body) = body {
        let call = issue_edit::build_body_edit_call(ctx, id, &body)?;
        if global.dry_run {
            payload.actions.push(PlannedAction::new("edit-body", &call));
        } else {
            runner.run(&call)?;
            payload.written = true;
        }
    }
    Ok(payload)
}

fn run_draft(
    global: &GlobalFlags,
    path: &str,
    write: bool,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    if write && path == "-" {
        return Err(ForgeError::validation(
            schema_err(),
            "tracker_stdin_not_writable",
            "--write needs a file to rewrite; --body-file - reads stdin",
            None,
        ));
    }
    let draft = read_draft(path)?;
    let (mut payload, body) = plan(TrackerTarget::draft(), &draft, write, global.dry_run)?;
    if let Some(body) = body
        && !global.dry_run
    {
        fs::write(path, body).map_err(|e| {
            ForgeError::software(
                schema_err(),
                format!("failed to write --body-file '{path}'"),
                Some(e.to_string()),
            )
        })?;
        payload.written = true;
    }
    Ok(emit(payload, write, format))
}

/// Generate the graph for `body` and, when `write` changes the body, return
/// the new body next to the payload.
///
/// A write needs a `## Phase table` section: without one the body is not a
/// tracker, and a mistyped issue id must not append a graph section to an
/// unrelated issue. Reading such a body still yields the zero-row graph.
fn plan(
    target: TrackerTarget,
    body: &str,
    write: bool,
    dry_run: bool,
) -> Result<(IssueTrackerGraphPayload, Option<String>), ForgeError> {
    if write && !tracker::has_phase_table(body) {
        return Err(ForgeError::validation(
            schema_err(),
            "tracker_no_phase_table",
            format!(
                "{} has no `## Phase table` section; --write only edits a tracker",
                target.describe()
            ),
            None,
        ));
    }
    let table = tracker::parse(body);
    let findings: Vec<TrackerFinding> = tracker::row_findings(&table)
        .iter()
        .map(TrackerFinding::from)
        .collect();
    let mut payload = IssueTrackerGraphPayload {
        target,
        graph: None,
        current: false,
        change: GraphChange::None.as_str(),
        changed: false,
        written: false,
        dry_run,
        findings,
        actions: Vec::new(),
    };
    if !payload.findings.is_empty() {
        return Ok((payload, None));
    }
    let graph = tracker::generate(&table.rows);
    let edit = write_graph(body, &graph);
    payload.graph = Some(graph.join("\n"));
    payload.current = edit.change == GraphChange::None;
    if !write || payload.current {
        return Ok((payload, None));
    }
    payload.change = edit.change.as_str();
    payload.changed = true;
    Ok((payload, Some(edit.body)))
}

fn emit(payload: IssueTrackerGraphPayload, write: bool, format: OutputFormat) -> i32 {
    if payload.findings.is_empty() {
        return emit_success(schema(), payload, format, |p| render_text(p, write));
    }
    let findings = payload.findings.clone();
    emit_findings(schema(), payload, &findings, format, |p| {
        render_text(p, write)
    })
}

fn schema() -> String {
    schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION)
}

fn render_text(payload: &IssueTrackerGraphPayload, write: bool) {
    let Some(graph) = &payload.graph else {
        render_findings(&payload.findings);
        return;
    };
    if !write {
        println!("```mermaid\n{graph}\n```");
        return;
    }
    let target = payload.target.describe();
    if !payload.changed {
        println!("dependency graph of {target} is already current");
    } else if payload.written {
        println!(
            "updated dependency graph of {target} ({change})",
            change = payload.change
        );
    } else {
        println!(
            "would update dependency graph of {target} ({change})",
            change = payload.change
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::issue_tracker::testing::{FakeForge, ctx, global};
    use crate::tracker::edit::write_graph;
    use pretty_assertions::assert_eq;

    const TABLE: &str =
        "## Phase table\n\n- [x] **A1** First: #11\n- [ ] **REL** Release · after A1\n";
    const GRAPH: &str = "graph LR\n  A1\n  REL{{REL}}\n  A1 --> REL";
    const STALE: &str = "## Phase table\n\n- [x] **A1** First: #11\n- [ ] **REL** Release · after A1\n\n## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n```\n\n## Notes\n\nKept.\n";

    fn lines() -> Vec<String> {
        GRAPH.split('\n').map(str::to_string).collect()
    }

    #[test]
    fn prints_the_graph_without_writing() {
        let forge = FakeForge::with_tracker(STALE);
        let payload = compute(&forge, &global(false), &ctx(), 1, false).expect("graph");
        assert_eq!(payload.graph.as_deref(), Some(GRAPH));
        assert!(!payload.current);
        assert!(!payload.changed);
        assert!(!payload.written);
        assert_eq!(payload.change, "none");
        assert_eq!(payload.findings, Vec::new());
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), STALE);
    }

    #[test]
    fn write_replaces_only_the_block_of_the_fresh_body() {
        let forge = FakeForge::with_tracker(STALE);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("graph");
        assert_eq!(payload.change, "replaced-block");
        assert!(payload.changed);
        assert!(payload.written);
        assert!(!payload.dry_run);
        assert!(payload.actions.is_empty());
        assert_eq!(forge.log(), ["view 1", "edit 1"]);
        assert_eq!(forge.body(1), write_graph(STALE, &lines()).body);
        assert_eq!(
            forge.body(1),
            STALE.replace(
                "graph LR\n  A1\n```",
                "graph LR\n  A1\n  REL{{REL}}\n  A1 --> REL\n```"
            )
        );
    }

    #[test]
    fn write_inserts_a_missing_section() {
        let forge = FakeForge::with_tracker(TABLE);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("graph");
        assert_eq!(payload.change, "inserted-section");
        assert_eq!(
            forge.body(1),
            format!("{TABLE}\n## Dependency graph\n\n```mermaid\n{GRAPH}\n```\n")
        );
    }

    #[test]
    fn write_is_skipped_when_the_block_is_current() {
        let current = write_graph(STALE, &lines()).body;
        let forge = FakeForge::with_tracker(&current);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("graph");
        assert!(payload.current);
        assert!(!payload.changed);
        assert!(!payload.written);
        assert_eq!(payload.change, "none");
        assert_eq!(forge.log(), ["view 1"]);
    }

    #[test]
    fn dry_run_plans_the_write_without_running_it() {
        let forge = FakeForge::with_tracker(STALE);
        let payload = compute(&forge, &global(true), &ctx(), 1, true).expect("graph");
        assert!(payload.changed);
        assert!(!payload.written);
        assert!(payload.dry_run);
        assert_eq!(payload.change, "replaced-block");
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), STALE);
        assert_eq!(payload.actions.len(), 1);
        assert_eq!(payload.actions[0].kind, "edit-body");
        let plan = &payload.actions[0].plan;
        let at = plan.iter().position(|arg| arg == "--body").expect("--body");
        assert_eq!(plan[at + 1], write_graph(STALE, &lines()).body);
    }

    #[test]
    fn refuses_a_table_with_row_findings() {
        let body = STALE.replace("· after A1", "· after A9");
        let forge = FakeForge::with_tracker(&body);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("graph");
        assert_eq!(payload.graph, None);
        assert_eq!(payload.findings.len(), 1);
        assert_eq!(payload.findings[0].code, "unknown-dependency");
        assert!(!payload.changed);
        assert!(!payload.written);
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), body);
    }

    #[test]
    fn write_refuses_a_body_without_a_phase_table() {
        // A mistyped id must not append a graph section to an unrelated issue.
        let unrelated = "An unrelated issue.\n\n## Notes\n\n- [ ] **A1** Not a tracker row: #2\n";
        for dry_run in [false, true] {
            let forge = FakeForge::with_tracker(unrelated);
            let err = compute(&forge, &global(dry_run), &ctx(), 1, true).expect_err("refused");
            assert_eq!(err.kind(), "tracker_no_phase_table");
            assert_eq!(err.exit_code(), nils_common::cli_contract::exit::DATA);
            assert_eq!(forge.log(), ["view 1"]);
            assert_eq!(forge.body(1), unrelated);
        }

        // Reading keeps the grammar's zero-row graph.
        let forge = FakeForge::with_tracker(unrelated);
        let payload = compute(&forge, &global(false), &ctx(), 1, false).expect("graph");
        assert_eq!(payload.graph.as_deref(), Some("graph LR"));
        assert!(!payload.current);
        assert_eq!(forge.log(), ["view 1"]);
    }

    #[test]
    fn write_fills_a_phase_table_that_has_no_rows() {
        let placeholder = "## Phase table\n\nNothing planned yet.\n";
        let forge = FakeForge::with_tracker(placeholder);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("graph");
        assert_eq!(payload.change, "inserted-section");
        assert!(payload.written);
        assert_eq!(
            forge.body(1),
            format!("{placeholder}\n## Dependency graph\n\n```mermaid\ngraph LR\n```\n")
        );
    }

    #[test]
    fn refuses_a_table_over_the_row_limit() {
        let mut body = String::from("## Phase table\n");
        for n in 1..=crate::tracker::MAX_ROWS + 1 {
            body.push_str(&format!("- [ ] **A{n}** Row {n}: #{n}\n"));
        }
        let forge = FakeForge::with_tracker(&body);
        let payload = compute(&forge, &global(false), &ctx(), 1, true).expect("graph");
        assert_eq!(payload.graph, None);
        let codes: Vec<&str> = payload.findings.iter().map(|f| f.code).collect();
        assert_eq!(codes, ["too-many-rows"]);
        assert!(!payload.changed);
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), body);
    }

    #[test]
    fn write_derives_from_the_read_immediately_before_it() {
        let forge = FakeForge::with_tracker(STALE);
        // Another session edits the issue before every read.
        let revision = std::cell::Cell::new(0);
        forge.before_view(move |issue| {
            revision.set(revision.get() + 1);
            issue.body.push_str(&format!("\nrev {}\n", revision.get()));
        });
        compute(&forge, &global(false), &ctx(), 1, true).expect("graph");
        let log = forge.log();
        let edit_at = log.iter().position(|call| call == "edit 1").expect("edit");
        assert_eq!(log[edit_at - 1], "view 1", "{log:?}");
        let fresh = forge.served.borrow()[edit_at - 1].clone();
        assert!(fresh.contains("\nrev "), "{fresh}");
        assert_eq!(forge.edited_body(0), write_graph(&fresh, &lines()).body);
    }

    #[test]
    fn write_applies_the_issue_body_guards() {
        let body = format!("{STALE}\nLogs are under /Users/dev/Project/secret.\n");
        let forge = FakeForge::with_tracker(&body);
        let err = compute(&forge, &global(false), &ctx(), 1, true).expect_err("guard");
        assert_eq!(err.kind(), "local_path_present");
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), body);
    }
}
