//! `issue tracker tick` atom.
//!
//! Spec / ops: `cli.forge-cli.issue.tracker.tick.v1`. Sets one phase-table
//! row's checkbox to `[x]` through [`crate::tracker::edit::tick`], which
//! changes only that row line; `--pr` records the delivering PR in the row's
//! notes and `--comment-file` posts one comment after the body write
//! succeeds. The graph block is not touched: it carries no done state.
//!
//! The issue is read immediately before the write and the write is a
//! transformation of that read, so a row another session changed since this
//! caller last looked is kept. The comment is validated before anything is
//! written; if posting it still fails after the body write, the error is
//! `tracker_comment_not_posted`. A row that is already ticked with nothing to
//! record writes nothing and posts nothing.

use std::fs;
use std::io::Read as _;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendRunner};
use crate::cli::{BINARY, GlobalFlags, IssueTrackerTickArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::issue_tracker::{PlannedAction, read_issue, schema_err};
use crate::ops::{issue_comment, issue_edit};
use crate::provider::{ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;
use crate::tracker::edit::{self, TickError};

const SCHEMA: &str = "issue.tracker.tick";
const SCHEMA_VERSION: u32 = 1;

/// Envelope payload for `cli.forge-cli.issue.tracker.tick.v1`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IssueTrackerTickPayload {
    pub provider: &'static str,
    pub number: u64,
    pub url: String,
    pub item: String,
    /// 1-based line of the row in the issue body.
    pub line: usize,
    pub row_before: String,
    pub row_after: String,
    /// Whether the row line changes.
    pub changed: bool,
    /// Whether the change was written (never under `--dry-run`).
    pub written: bool,
    pub dry_run: bool,
    pub comment_posted: bool,
    pub comment_url: Option<String>,
    /// Under `--dry-run`, the backend calls a real run would make.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PlannedAction>,
}

pub fn run(
    global: &GlobalFlags,
    args: IssueTrackerTickArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    if global.is_local() {
        let runner = crate::local::LocalRunner::from_global(global)?;
        return run_with(&runner, global, args, format, git_remote_url);
    }
    let runner = default_runner();
    run_with(&runner, global, args, format, git_remote_url)
}

pub fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    args: IssueTrackerTickArgs,
    format: OutputFormat,
    remote_url_lookup: F,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        remote_url_lookup,
    )?;
    let payload = compute(runner, global, &ctx, &args)?;
    Ok(emit_success(
        schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
        payload,
        format,
        render_text,
    ))
}

/// Validate the inputs, read the tracker issue, and write the ticked row into
/// that freshly read body.
pub fn compute<R: BackendRunner>(
    runner: &R,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    args: &IssueTrackerTickArgs,
) -> Result<IssueTrackerTickPayload, ForgeError> {
    if args
        .pr
        .as_deref()
        .is_some_and(|pr| !edit::pr_is_recordable(pr))
    {
        return Err(refusal(&args.item, TickError::InvalidPr));
    }
    let comment = match args.comment_file.as_deref() {
        Some(path) => Some(comment_call(ctx, args.id, path)?),
        None => None,
    };

    let view = read_issue(runner, ctx, args.id)?;
    let tick = edit::tick(&view.body, &args.item, args.pr.as_deref())
        .map_err(|err| refusal(&args.item, err))?;
    let mut payload = IssueTrackerTickPayload {
        provider: view.provider,
        number: view.number,
        url: view.url,
        item: args.item.clone(),
        line: tick.line,
        changed: tick.changed(),
        row_before: tick.before,
        row_after: tick.after,
        written: false,
        dry_run: global.dry_run,
        comment_posted: false,
        comment_url: None,
        actions: Vec::new(),
    };
    if !payload.changed {
        return Ok(payload);
    }

    let write = issue_edit::build_body_edit_call(ctx, args.id, &tick.body)?;
    if global.dry_run {
        payload
            .actions
            .push(PlannedAction::new("edit-body", &write));
        if let Some(comment) = &comment {
            payload.actions.push(PlannedAction::new("comment", comment));
        }
        return Ok(payload);
    }
    runner.run(&write)?;
    payload.written = true;
    if let Some(comment) = &comment {
        // The row is already ticked, so a rerun would be a no-op that posts
        // nothing: say so instead of surfacing a bare backend error.
        let posted = runner.run(comment).map_err(|err| {
            ForgeError::runtime_failure(
                schema_err(),
                "tracker_comment_not_posted",
                format!(
                    "{item} was ticked but the comment was not posted; post it with `issue comment`",
                    item = args.item
                ),
                Some(format!("{}: {}", err.kind(), err.message())),
            )
        })?;
        payload.comment_posted = true;
        payload.comment_url = issue_comment::first_url(&posted.stdout);
    }
    Ok(payload)
}

/// Read `--comment-file` (`-` is stdin) and build the guarded comment call.
/// The guards and error kinds are those of `issue comment`; the messages name
/// this command's flag instead of `--body` / `--body-file`.
fn comment_call(ctx: &ProviderContext, id: u64, path: &str) -> Result<BackendCall, ForgeError> {
    let body = if path == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            ForgeError::software(
                schema_err(),
                "failed to read --comment-file from stdin",
                Some(e.to_string()),
            )
        })?;
        buf
    } else {
        fs::read_to_string(path).map_err(|e| {
            ForgeError::software(
                schema_err(),
                format!("failed to read --comment-file '{path}'"),
                Some(e.to_string()),
            )
        })?
    };
    issue_comment::build_guarded_comment_call(ctx, id, &body).map_err(|err| {
        if err.kind() == "body_missing_summary" {
            ForgeError::validation(
                schema_err(),
                "body_missing_summary",
                "comment body is empty (--comment-file has no text)",
                None,
            )
        } else {
            err
        }
    })
}

fn refusal(item: &str, err: TickError) -> ForgeError {
    let lines = |lines: &[usize]| {
        let list: Vec<String> = lines.iter().map(usize::to_string).collect();
        let noun = if lines.len() == 1 { "line" } else { "lines" };
        Some(format!("{noun} {}", list.join(", ")))
    };
    let (kind, message, detail) = match err {
        TickError::UnknownItem => (
            "tracker_item_unknown",
            format!("no phase-table row has the id {item}"),
            None,
        ),
        TickError::DuplicateItem { lines: at } => (
            "tracker_item_duplicated",
            format!("more than one phase-table row has the id {item}"),
            lines(&at),
        ),
        TickError::MalformedRow { lines: at } => (
            "tracker_item_malformed",
            format!("the row for {item} does not match the tracker row grammar"),
            lines(&at),
        ),
        TickError::TooManyRows => (
            "tracker_too_many_rows",
            format!(
                "the phase table has more than {} rows and was not analysed",
                crate::tracker::MAX_ROWS
            ),
            None,
        ),
        TickError::InvalidPr => (
            "tracker_pr_invalid",
            "--pr must be one reference without whitespace, parentheses, commas, or a middle dot"
                .to_string(),
            None,
        ),
    };
    ForgeError::validation(schema_err(), kind, message, detail)
}

fn render_text(payload: &IssueTrackerTickPayload) {
    let place = format!(
        "{provider} issue #{number} (line {line})",
        provider = payload.provider,
        number = payload.number,
        line = payload.line,
    );
    if !payload.changed {
        println!(
            "{item} is already ticked in {place}; nothing to change",
            item = payload.item
        );
        return;
    }
    let verb = if payload.written {
        "ticked"
    } else {
        "would tick"
    };
    println!(
        "{verb} {item} in {place}: {row}",
        item = payload.item,
        row = payload.row_after,
    );
    if let Some(url) = &payload.comment_url {
        println!("commented: {url}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::issue_tracker::testing::{FakeForge, ctx, global};
    use pretty_assertions::assert_eq;
    use std::io::Write as _;

    const TABLE: &str = "## Phase table\n\n- [ ] **A1** First: #11\n- [ ] **A2** Second: #12 · after A1\n\n## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n  A2\n  A1 --> A2\n```\n";

    fn args(item: &str, pr: Option<&str>, comment_file: Option<&str>) -> IssueTrackerTickArgs {
        IssueTrackerTickArgs {
            id: 1,
            item: item.into(),
            pr: pr.map(str::to_string),
            comment_file: comment_file.map(str::to_string),
        }
    }

    fn comment_file(text: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("comment file");
        file.write_all(text.as_bytes()).expect("write comment");
        file
    }

    #[test]
    fn ticks_the_row_in_the_fresh_body() {
        let forge = FakeForge::with_tracker(TABLE);
        let payload =
            compute(&forge, &global(false), &ctx(), &args("A1", None, None)).expect("tick");
        assert_eq!(payload.provider, "github");
        assert_eq!(payload.number, 1);
        assert_eq!(payload.item, "A1");
        assert_eq!(payload.line, 3);
        assert_eq!(payload.row_before, "- [ ] **A1** First: #11");
        assert_eq!(payload.row_after, "- [x] **A1** First: #11");
        assert!(payload.changed);
        assert!(payload.written);
        assert!(!payload.comment_posted);
        assert_eq!(payload.comment_url, None);
        assert_eq!(forge.log(), ["view 1", "edit 1"]);
        assert_eq!(forge.body(1), TABLE.replace("- [ ] **A1**", "- [x] **A1**"));
    }

    #[test]
    fn records_the_pr_and_posts_the_comment_after_the_body_write() {
        let forge = FakeForge::with_tracker(TABLE);
        let comment = comment_file("Delivered A2 in #30.\n");
        let payload = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A2", Some("#30"), comment.path().to_str()),
        )
        .expect("tick");
        assert_eq!(
            payload.row_after,
            "- [x] **A2** Second: #12 (PR #30) · after A1"
        );
        assert!(payload.comment_posted);
        assert_eq!(
            payload.comment_url.as_deref(),
            Some("https://github.com/example/tracker/issues/1#issuecomment-9")
        );
        assert_eq!(forge.log(), ["view 1", "edit 1", "comment"]);
        assert_eq!(*forge.comments.borrow(), ["Delivered A2 in #30.\n"]);
        assert_eq!(
            forge.body(1),
            TABLE.replace(
                "- [ ] **A2** Second: #12 · after A1",
                "- [x] **A2** Second: #12 (PR #30) · after A1"
            )
        );
    }

    #[test]
    fn a_ticked_row_with_nothing_to_record_writes_and_posts_nothing() {
        let ticked = TABLE.replace("- [ ] **A1**", "- [x] **A1**");
        let forge = FakeForge::with_tracker(&ticked);
        let comment = comment_file("Delivered A1.\n");
        let payload = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A1", None, comment.path().to_str()),
        )
        .expect("tick");
        assert!(!payload.changed);
        assert!(!payload.written);
        assert!(!payload.comment_posted);
        assert_eq!(payload.row_before, payload.row_after);
        assert_eq!(forge.log(), ["view 1"]);
        assert!(forge.comments.borrow().is_empty());
        assert_eq!(forge.body(1), ticked);
    }

    #[test]
    fn dry_run_plans_the_edit_and_the_comment_without_running_them() {
        let forge = FakeForge::with_tracker(TABLE);
        let comment = comment_file("Delivered A1.\n");
        let payload = compute(
            &forge,
            &global(true),
            &ctx(),
            &args("A1", None, comment.path().to_str()),
        )
        .expect("tick");
        assert!(payload.changed);
        assert!(!payload.written);
        assert!(payload.dry_run);
        assert!(!payload.comment_posted);
        assert_eq!(payload.row_after, "- [x] **A1** First: #11");
        let kinds: Vec<&str> = payload.actions.iter().map(|action| action.kind).collect();
        assert_eq!(kinds, ["edit-body", "comment"]);
        let plan = &payload.actions[0].plan;
        let at = plan.iter().position(|arg| arg == "--body").expect("--body");
        assert_eq!(plan[at + 1], TABLE.replace("- [ ] **A1**", "- [x] **A1**"));
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), TABLE);
        assert!(forge.comments.borrow().is_empty());
    }

    #[test]
    fn refuses_an_unknown_duplicated_or_malformed_item() {
        let forge = FakeForge::with_tracker(TABLE);
        let err =
            compute(&forge, &global(false), &ctx(), &args("A9", None, None)).expect_err("unknown");
        assert_eq!(err.kind(), "tracker_item_unknown");

        let duplicated = TABLE.replace("**A2**", "**A1**");
        let forge = FakeForge::with_tracker(&duplicated);
        let err = compute(&forge, &global(false), &ctx(), &args("A1", None, None))
            .expect_err("duplicated");
        assert_eq!(err.kind(), "tracker_item_duplicated");
        assert_eq!(err.detail(), Some("lines 3, 4"));
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), duplicated);

        let malformed = TABLE.replace("· after A1", "· after A1 and more");
        let forge = FakeForge::with_tracker(&malformed);
        let err = compute(&forge, &global(false), &ctx(), &args("A2", None, None))
            .expect_err("malformed");
        assert_eq!(err.kind(), "tracker_item_malformed");
        assert_eq!(err.detail(), Some("line 4"));
        assert_eq!(forge.log(), ["view 1"]);
    }

    #[test]
    fn refuses_an_unusable_pr_reference_before_any_provider_call() {
        let forge = FakeForge::with_tracker(TABLE);
        let err = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A1", Some("#30 (draft)"), None),
        )
        .expect_err("pr");
        assert_eq!(err.kind(), "tracker_pr_invalid");
        assert!(forge.log().is_empty());
    }

    #[test]
    fn a_rejected_comment_stops_before_any_provider_call() {
        let forge = FakeForge::with_tracker(TABLE);
        let local_path = comment_file("Logs are under /Users/dev/Project/secret.\n");
        let err = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A1", None, local_path.path().to_str()),
        )
        .expect_err("local path");
        assert_eq!(err.kind(), "local_path_present");

        let blank = comment_file(" \n");
        let err = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A1", None, blank.path().to_str()),
        )
        .expect_err("blank");
        assert_eq!(err.kind(), "body_missing_summary");

        assert!(forge.log().is_empty());
        assert_eq!(forge.body(1), TABLE);
    }

    #[test]
    fn comment_file_errors_name_the_comment_file_flag() {
        let forge = FakeForge::with_tracker(TABLE);

        let missing = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A1", None, Some("no-such-dir/comment.md")),
        )
        .expect_err("missing file");
        assert_eq!(missing.kind(), "software_error");
        assert_eq!(
            missing.message(),
            "failed to read --comment-file 'no-such-dir/comment.md'"
        );

        let blank = comment_file(" \n");
        let empty = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A1", None, blank.path().to_str()),
        )
        .expect_err("blank file");
        assert_eq!(empty.kind(), "body_missing_summary");
        assert_eq!(
            empty.message(),
            "comment body is empty (--comment-file has no text)"
        );

        assert!(forge.log().is_empty());
    }

    #[test]
    fn refuses_a_table_over_the_row_limit() {
        let mut body = String::from("## Phase table\n");
        for n in 1..=crate::tracker::MAX_ROWS + 1 {
            body.push_str(&format!("- [ ] **A{n}** Row {n}: #{n}\n"));
        }
        let forge = FakeForge::with_tracker(&body);
        let err = compute(&forge, &global(false), &ctx(), &args("A1", None, None))
            .expect_err("too many rows");
        assert_eq!(err.kind(), "tracker_too_many_rows");
        assert_eq!(err.exit_code(), nils_common::cli_contract::exit::DATA);
        assert_eq!(forge.log(), ["view 1"]);
        assert_eq!(forge.body(1), body);
    }

    #[test]
    fn a_body_without_a_phase_table_has_no_item_to_tick() {
        let forge = FakeForge::with_tracker("An unrelated issue.\n- [ ] **A1** Not a row: #2\n");
        let err =
            compute(&forge, &global(false), &ctx(), &args("A1", None, None)).expect_err("no table");
        assert_eq!(err.kind(), "tracker_item_unknown");
        assert_eq!(err.message(), "no phase-table row has the id A1");
        assert_eq!(forge.log(), ["view 1"]);
    }

    #[test]
    fn a_comment_that_fails_after_the_write_says_the_row_was_ticked() {
        let forge = FakeForge::with_tracker(TABLE);
        forge.fail_comments.set(true);
        let comment = comment_file("Delivered A1.\n");
        let err = compute(
            &forge,
            &global(false),
            &ctx(),
            &args("A1", None, comment.path().to_str()),
        )
        .expect_err("comment outage");
        assert_eq!(err.kind(), "tracker_comment_not_posted");
        assert_eq!(err.exit_code(), nils_common::cli_contract::exit::RUNTIME);
        assert_eq!(err.detail(), Some("backend_error: gh exited with status 1"));
        // The body write landed; a rerun finds the row ticked.
        assert_eq!(forge.log(), ["view 1", "edit 1", "comment"]);
        assert_eq!(forge.body(1), TABLE.replace("- [ ] **A1**", "- [x] **A1**"));
    }

    #[test]
    fn write_derives_from_the_read_immediately_before_it() {
        let forge = FakeForge::with_tracker(TABLE);
        // Another session edits the issue before every read.
        let revision = std::cell::Cell::new(0);
        forge.before_view(move |issue| {
            revision.set(revision.get() + 1);
            issue.body.push_str(&format!("\nrev {}\n", revision.get()));
        });
        compute(&forge, &global(false), &ctx(), &args("A2", None, None)).expect("tick");
        let log = forge.log();
        let edit_at = log.iter().position(|call| call == "edit 1").expect("edit");
        assert_eq!(log[edit_at - 1], "view 1", "{log:?}");
        let fresh = forge.served.borrow()[edit_at - 1].clone();
        assert!(fresh.contains("\nrev "), "{fresh}");
        assert_eq!(
            forge.edited_body(0),
            fresh.replace("- [ ] **A2**", "- [x] **A2**")
        );
    }

    #[test]
    fn a_tick_made_by_another_session_since_the_last_read_is_kept() {
        let forge = FakeForge::with_tracker(TABLE);
        // The caller last saw TABLE; by the time this command reads, another
        // session has ticked A1 and added a note.
        forge.before_view(|issue| {
            issue.body = format!(
                "{}\nA note from another session.\n",
                TABLE.replace("- [ ] **A1**", "- [x] **A1**")
            );
        });
        compute(&forge, &global(false), &ctx(), &args("A2", None, None)).expect("tick");
        let body = forge.body(1);
        assert!(body.contains("- [x] **A1** First: #11"), "{body}");
        assert!(
            body.contains("- [x] **A2** Second: #12 · after A1"),
            "{body}"
        );
        assert!(body.ends_with("\nA note from another session.\n"), "{body}");
    }
}
