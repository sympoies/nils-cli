use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

use plan_tooling::parse::parse_plan_with_display;
use serde_json::{Value, json};

use crate::cli::Cli;
use crate::commands::build::{BuildPlanTaskSpecArgs, BuildTaskSpecArgs};
use crate::commands::plan::{
    CleanupWorktreesArgs, ClosePlanArgs, ReadyPlanArgs, StartPlanArgs, StatusPlanArgs,
};
use crate::commands::sprint::{
    AcceptSprintArgs, MultiSprintGuideArgs, ReadySprintArgs, StartSprintArgs,
};
use crate::commands::{Command as CliCommand, SummaryArgs};
use crate::github::{GhCliAdapter, GitHubAdapter};
use crate::issue_body::{self, TaskRow};
use crate::render::{self, SprintCommentInput, SprintCommentMode};
use crate::task_spec::{self, TaskSpecBuildOptions, TaskSpecRow, TaskSpecScope};
use crate::{BinaryFlavor, CommandError};

pub fn execute(binary: BinaryFlavor, cli: &Cli) -> Result<Value, CommandError> {
    match &cli.command {
        CliCommand::BuildTaskSpec(args) => run_build_task_spec(args),
        CliCommand::BuildPlanTaskSpec(args) => run_build_plan_task_spec(args),
        CliCommand::StartPlan(args) => {
            run_start_plan(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::StatusPlan(args) => {
            run_status_plan(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::ReadyPlan(args) => {
            run_ready_plan(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::ClosePlan(args) => {
            run_close_plan(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::CleanupWorktrees(args) => {
            run_cleanup_worktrees(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::StartSprint(args) => {
            run_start_sprint(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::ReadySprint(args) => {
            run_ready_sprint(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::AcceptSprint(args) => {
            run_accept_sprint(binary, cli.dry_run, cli.repo.as_deref(), args)
        }
        CliCommand::MultiSprintGuide(args) => run_multi_sprint_guide(args),
        CliCommand::Completion(_) => Err(CommandError::usage(
            "completion-direct-output-only",
            "completion output is emitted directly; run `<binary> completion <bash|zsh>`",
        )),
    }
}

fn run_build_task_spec(args: &BuildTaskSpecArgs) -> Result<Value, CommandError> {
    let options = to_build_options(
        args.prefixes.owner_prefix.clone(),
        args.prefixes.branch_prefix.clone(),
        args.prefixes.worktree_prefix.clone(),
        args.grouping.pr_grouping,
        args.grouping.strategy,
        args.grouping.pr_group.clone(),
    );
    let build = task_spec::build_task_spec(
        &args.plan,
        TaskSpecScope::Sprint(i32::from(args.sprint)),
        &options,
    )
    .map_err(|err| CommandError::runtime("task-spec-generation-failed", err))?;

    let out_path = args.task_spec_out.clone().unwrap_or_else(|| {
        task_spec::default_sprint_task_spec_path(&args.plan, i32::from(args.sprint))
    });
    task_spec::write_tsv(&out_path, &build.rows)
        .map_err(|err| CommandError::runtime("task-spec-write-failed", err))?;

    Ok(json!({
        "scope": "sprint",
        "sprint": args.sprint,
        "task_spec_path": path_text(&out_path),
        "record_count": build.rows.len(),
        "plan_title": build.plan_title,
    }))
}

fn run_build_plan_task_spec(args: &BuildPlanTaskSpecArgs) -> Result<Value, CommandError> {
    let options = to_build_options(
        args.prefixes.owner_prefix.clone(),
        args.prefixes.branch_prefix.clone(),
        args.prefixes.worktree_prefix.clone(),
        args.grouping.pr_grouping,
        args.grouping.strategy,
        args.grouping.pr_group.clone(),
    );
    let build = task_spec::build_task_spec(&args.plan, TaskSpecScope::Plan, &options)
        .map_err(|err| CommandError::runtime("task-spec-generation-failed", err))?;

    let out_path = args
        .task_spec_out
        .clone()
        .unwrap_or_else(|| task_spec::default_plan_task_spec_path(&args.plan));
    task_spec::write_tsv(&out_path, &build.rows)
        .map_err(|err| CommandError::runtime("task-spec-write-failed", err))?;

    Ok(json!({
        "scope": "plan",
        "task_spec_path": path_text(&out_path),
        "record_count": build.rows.len(),
        "plan_title": build.plan_title,
    }))
}

fn run_start_plan(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &StartPlanArgs,
) -> Result<Value, CommandError> {
    let options = to_build_options(
        args.prefixes.owner_prefix.clone(),
        args.prefixes.branch_prefix.clone(),
        args.prefixes.worktree_prefix.clone(),
        args.grouping.pr_grouping,
        args.grouping.strategy,
        args.grouping.pr_group.clone(),
    );

    let build = task_spec::build_task_spec(&args.plan, TaskSpecScope::Plan, &options)
        .map_err(|err| CommandError::runtime("task-spec-generation-failed", err))?;

    let task_spec_out = args
        .task_spec_out
        .clone()
        .unwrap_or_else(|| task_spec::default_plan_task_spec_path(&args.plan));
    task_spec::write_tsv(&task_spec_out, &build.rows)
        .map_err(|err| CommandError::runtime("task-spec-write-failed", err))?;

    let issue_body_out = args
        .issue_body_out
        .clone()
        .unwrap_or_else(|| render::default_plan_issue_body_path(&args.plan));

    let plan_title = args
        .title
        .clone()
        .unwrap_or_else(|| build.plan_title.clone());

    let issue_body = render::render_plan_issue_body(
        &args.plan,
        &build.display_plan_path,
        &plan_title,
        &build.rows,
    );
    render::write_rendered(&issue_body_out, &issue_body)
        .map_err(|err| CommandError::runtime("issue-body-write-failed", err))?;

    let mut issue_number: Option<u64> = None;
    let mut issue_url: Option<String> = None;
    let mut live_mutations = false;

    if binary == BinaryFlavor::PlanIssue && !dry_run {
        let repo = resolve_repo_for_live(binary, repo_override)?;
        let adapter = GhCliAdapter;
        let (number, url) = adapter
            .create_issue(&repo, &plan_title, &issue_body_out, &args.label)
            .map_err(|err| CommandError::runtime("github-issue-create-failed", err))?;
        issue_number = Some(number);
        issue_url = Some(url);
        live_mutations = true;
    }

    Ok(json!({
        "scope": "plan",
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "task_spec_path": path_text(&task_spec_out),
        "issue_body_path": path_text(&issue_body_out),
        "record_count": build.rows.len(),
        "issue_number": issue_number,
        "issue_url": issue_url,
        "labels": args.label,
        "live_mutations_performed": live_mutations,
    }))
}

fn run_status_plan(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &StatusPlanArgs,
) -> Result<Value, CommandError> {
    let adapter = GhCliAdapter;

    let (body, issue, repo, source) = if let Some(path) = &args.body_file {
        let body = fs::read_to_string(path).map_err(|err| {
            CommandError::runtime(
                "issue-body-read-failed",
                format!("failed to read body file {}: {err}", path.display()),
            )
        })?;
        (body, None, None, format!("body-file:{}", path.display()))
    } else {
        let issue = args
            .issue
            .ok_or_else(|| CommandError::usage("missing-issue", "--issue is required"))?;
        ensure_live_binary(binary)?;
        let repo = resolve_repo_for_live(binary, repo_override)?;
        let body = adapter
            .issue_body(&repo, issue)
            .map_err(|err| CommandError::runtime("github-issue-read-failed", err))?;
        (body, Some(issue), Some(repo), format!("issue:{issue}"))
    };

    let table = issue_body::parse_task_table(&body)
        .map_err(|err| CommandError::runtime("issue-body-parse-failed", err))?;

    let structure_errors = issue_body::validate_rows(table.rows());
    if !structure_errors.is_empty() {
        return Err(CommandError::runtime(
            "issue-body-invalid",
            structure_errors.join(" | "),
        ));
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in table.rows() {
        let status = row.status.trim().to_ascii_lowercase();
        *counts.entry(status).or_insert(0) += 1;
    }

    let should_comment = args.comment_mode.comment && !args.comment_mode.no_comment;
    let comment_text = render_plan_status_comment(table.rows());
    let mut live_mutations = false;

    if should_comment
        && binary == BinaryFlavor::PlanIssue
        && !dry_run
        && let (Some(issue), Some(repo)) = (issue, repo.as_deref())
    {
        let comment_path = write_temp_markdown("status-plan-comment", &comment_text)
            .map_err(|err| CommandError::runtime("comment-write-failed", err))?;
        adapter
            .comment_issue(repo, issue, &comment_path)
            .map_err(|err| CommandError::runtime("github-comment-failed", err))?;
        live_mutations = true;
    }

    Ok(json!({
        "scope": "plan",
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "issue_source": source,
        "task_count": table.rows().len(),
        "status_counts": counts,
        "comment_requested": should_comment,
        "comment_preview": should_comment.then_some(comment_text),
        "live_mutations_performed": live_mutations,
    }))
}

fn run_ready_plan(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &ReadyPlanArgs,
) -> Result<Value, CommandError> {
    let adapter = GhCliAdapter;

    let (body, issue, repo, source) = if let Some(path) = &args.body_file {
        let body = fs::read_to_string(path).map_err(|err| {
            CommandError::runtime(
                "issue-body-read-failed",
                format!("failed to read body file {}: {err}", path.display()),
            )
        })?;
        (body, None, None, format!("body-file:{}", path.display()))
    } else {
        let issue = args
            .issue
            .ok_or_else(|| CommandError::usage("missing-issue", "--issue is required"))?;
        ensure_live_binary(binary)?;
        let repo = resolve_repo_for_live(binary, repo_override)?;
        let body = adapter
            .issue_body(&repo, issue)
            .map_err(|err| CommandError::runtime("github-issue-read-failed", err))?;
        (body, Some(issue), Some(repo), format!("issue:{issue}"))
    };

    let table = issue_body::parse_task_table(&body)
        .map_err(|err| CommandError::runtime("issue-body-parse-failed", err))?;
    let structure_errors = issue_body::validate_rows(table.rows());
    if !structure_errors.is_empty() {
        return Err(CommandError::runtime(
            "issue-body-invalid",
            structure_errors.join(" | "),
        ));
    }

    let summary = load_summary(&args.summary)?;
    let should_comment = !args.comment_mode.no_comment;
    let comment_text = summary.unwrap_or_else(|| "Final plan review requested.".to_string());

    let mut labels_updated = false;
    let mut comment_posted = false;
    let mut live_mutations = false;

    if binary == BinaryFlavor::PlanIssue
        && !dry_run
        && let (Some(issue), Some(repo)) = (issue, repo.as_deref())
    {
        if !args.no_label_update {
            adapter
                .edit_issue_labels(
                    repo,
                    issue,
                    std::slice::from_ref(&args.label),
                    &args.remove_label,
                )
                .map_err(|err| CommandError::runtime("github-label-update-failed", err))?;
            labels_updated = true;
            live_mutations = true;
        }

        if should_comment {
            let comment_path = write_temp_markdown("ready-plan-comment", &comment_text)
                .map_err(|err| CommandError::runtime("comment-write-failed", err))?;
            adapter
                .comment_issue(repo, issue, &comment_path)
                .map_err(|err| CommandError::runtime("github-comment-failed", err))?;
            comment_posted = true;
            live_mutations = true;
        }
    }

    Ok(json!({
        "scope": "plan",
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "issue_source": source,
        "task_count": table.rows().len(),
        "summary": comment_text,
        "label": args.label,
        "remove_label": args.remove_label,
        "label_update_requested": !args.no_label_update,
        "label_update_applied": labels_updated,
        "comment_requested": should_comment,
        "comment_posted": comment_posted,
        "live_mutations_performed": live_mutations,
    }))
}

fn run_close_plan(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &ClosePlanArgs,
) -> Result<Value, CommandError> {
    if !approval_comment_url_looks_valid(&args.approved_comment_url) {
        return Err(CommandError::usage(
            "invalid-approval-comment-url",
            "--approved-comment-url must be a GitHub issue/pull comment URL",
        ));
    }

    let adapter = GhCliAdapter;
    let close_comment = load_close_comment(&args.close_comment)?;

    let (body, issue, repo, source) = if let Some(path) = &args.body_file {
        let body = fs::read_to_string(path).map_err(|err| {
            CommandError::runtime(
                "issue-body-read-failed",
                format!("failed to read body file {}: {err}", path.display()),
            )
        })?;
        let repo = (binary == BinaryFlavor::PlanIssue)
            .then(|| resolve_repo_for_live(binary, repo_override))
            .transpose()?;
        (
            body,
            args.issue,
            repo,
            format!("body-file:{}", path.display()),
        )
    } else {
        let issue = args
            .issue
            .ok_or_else(|| CommandError::usage("missing-issue", "--issue is required"))?;
        ensure_live_binary(binary)?;
        let repo = resolve_repo_for_live(binary, repo_override)?;
        let body = adapter
            .issue_body(&repo, issue)
            .map_err(|err| CommandError::runtime("github-issue-read-failed", err))?;
        (body, Some(issue), Some(repo), format!("issue:{issue}"))
    };

    let table = issue_body::parse_task_table(&body)
        .map_err(|err| CommandError::runtime("issue-body-parse-failed", err))?;

    let mut gate_errors = issue_body::validate_rows(table.rows());

    if !args.allow_not_done {
        for row in table.rows() {
            if !row.status.trim().eq_ignore_ascii_case("done") {
                gate_errors.push(format!(
                    "{}: close gate requires Status=done (found `{}`)",
                    row.task,
                    row.status.trim()
                ));
            }
        }
    }

    let required_prs = collect_required_prs(table.rows(), "close-plan")
        .map_err(|err| CommandError::runtime("close-gate-failed", err))?;

    let mut merge_checks_skipped = false;
    if let Some(repo) = repo.as_deref() {
        ensure_prs_merged(&adapter, repo, &required_prs, "close-plan")
            .map_err(|err| CommandError::runtime("close-gate-failed", err))?;
    } else {
        merge_checks_skipped = true;
    }

    if !gate_errors.is_empty() {
        return Err(CommandError::runtime(
            "close-gate-failed",
            gate_errors.join(" | "),
        ));
    }

    let cleanup = cleanup_worktrees_from_rows(table.rows(), dry_run)
        .map_err(|err| CommandError::runtime("worktree-cleanup-failed", err))?;

    let mut issue_closed = false;
    let mut live_mutations = false;

    if binary == BinaryFlavor::PlanIssue && !dry_run {
        let issue = issue.ok_or_else(|| {
            CommandError::usage("missing-issue", "--issue is required for live close-plan")
        })?;
        let repo = repo.as_deref().ok_or_else(|| {
            CommandError::usage(
                "missing-repo",
                "unable to resolve repository for live close-plan",
            )
        })?;

        adapter
            .close_issue(repo, issue, args.reason, close_comment.as_deref())
            .map_err(|err| CommandError::runtime("github-issue-close-failed", err))?;
        issue_closed = true;
        live_mutations = true;
    }

    Ok(json!({
        "scope": "plan",
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "issue_source": source,
        "approval_comment_url": args.approved_comment_url,
        "allow_not_done": args.allow_not_done,
        "issue_closed": issue_closed,
        "close_comment_applied": close_comment.as_ref().is_some_and(|v| !v.trim().is_empty()),
        "cleanup": {
            "targeted": cleanup.targeted,
            "removed": cleanup.removed,
            "residual": cleanup.residual,
        },
        "merge_checks_skipped": merge_checks_skipped,
        "live_mutations_performed": live_mutations,
    }))
}

fn run_cleanup_worktrees(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &CleanupWorktreesArgs,
) -> Result<Value, CommandError> {
    ensure_live_binary(binary)?;

    let repo = resolve_repo_for_live(binary, repo_override)?;
    let adapter = GhCliAdapter;
    let body = adapter
        .issue_body(&repo, args.issue)
        .map_err(|err| CommandError::runtime("github-issue-read-failed", err))?;

    let table = issue_body::parse_task_table(&body)
        .map_err(|err| CommandError::runtime("issue-body-parse-failed", err))?;
    let structure_errors = issue_body::validate_rows(table.rows());
    if !structure_errors.is_empty() {
        return Err(CommandError::runtime(
            "issue-body-invalid",
            structure_errors.join(" | "),
        ));
    }

    let cleanup = cleanup_worktrees_from_rows(table.rows(), dry_run)
        .map_err(|err| CommandError::runtime("worktree-cleanup-failed", err))?;

    Ok(json!({
        "scope": "plan",
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "issue": args.issue,
        "cleanup": {
            "targeted": cleanup.targeted,
            "removed": cleanup.removed,
            "residual": cleanup.residual,
        },
        "live_mutations_performed": !dry_run && !cleanup.removed.is_empty(),
    }))
}

fn run_start_sprint(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &StartSprintArgs,
) -> Result<Value, CommandError> {
    let options = to_build_options(
        args.prefixes.owner_prefix.clone(),
        args.prefixes.branch_prefix.clone(),
        args.prefixes.worktree_prefix.clone(),
        args.grouping.pr_grouping,
        args.grouping.strategy,
        args.grouping.pr_group.clone(),
    );

    let build = task_spec::build_task_spec(
        &args.plan,
        TaskSpecScope::Sprint(i32::from(args.sprint)),
        &options,
    )
    .map_err(|err| CommandError::runtime("task-spec-generation-failed", err))?;

    let task_spec_out = args.task_spec_out.clone().unwrap_or_else(|| {
        task_spec::default_sprint_task_spec_path(&args.plan, i32::from(args.sprint))
    });
    task_spec::write_tsv(&task_spec_out, &build.rows)
        .map_err(|err| CommandError::runtime("task-spec-write-failed", err))?;

    let prompts_out = args
        .subagent_prompts_out
        .clone()
        .unwrap_or_else(|| default_subagent_prompts_path(&args.plan, i32::from(args.sprint)));
    let prompt_files = write_subagent_prompts(
        &prompts_out,
        args.issue,
        i32::from(args.sprint),
        &build.rows,
    )
    .map_err(|err| CommandError::runtime("subagent-prompt-write-failed", err))?;

    let sprint_name = build
        .sprint_name
        .clone()
        .unwrap_or_else(|| format!("Sprint {}", args.sprint));

    let adapter = GhCliAdapter;
    let mut issue_body_for_comment: Option<String> = None;
    let mut synced_rows = 0usize;
    let mut live_mutations = false;

    if binary == BinaryFlavor::PlanIssue {
        let repo = resolve_repo_for_live(binary, repo_override)?;
        let body = adapter
            .issue_body(&repo, args.issue)
            .map_err(|err| CommandError::runtime("github-issue-read-failed", err))?;

        let mut table = issue_body::parse_task_table(&body)
            .map_err(|err| CommandError::runtime("issue-body-parse-failed", err))?;

        let structure_errors = issue_body::validate_rows(table.rows());
        if !structure_errors.is_empty() {
            return Err(CommandError::runtime(
                "issue-body-invalid",
                structure_errors.join(" | "),
            ));
        }

        if args.sprint > 1 {
            enforce_previous_sprint_gate(&adapter, &repo, table.rows(), i32::from(args.sprint))
                .map_err(|err| CommandError::runtime("previous-sprint-gate-failed", err))?;
        }

        synced_rows = sync_issue_rows_from_task_spec(&mut table, &build.rows)
            .map_err(|err| CommandError::runtime("task-sync-failed", err))?;

        let updated_body = table.render();
        issue_body_for_comment = Some(updated_body.clone());

        if !dry_run {
            let body_path = write_temp_markdown("start-sprint-issue-body", &updated_body)
                .map_err(|err| CommandError::runtime("issue-body-write-failed", err))?;
            adapter
                .edit_issue_body(&repo, args.issue, &body_path)
                .map_err(|err| CommandError::runtime("github-issue-update-failed", err))?;
            live_mutations = true;
        }
    }

    let comment = render::render_sprint_comment(SprintCommentInput {
        mode: SprintCommentMode::Start,
        plan_file: &args.plan,
        sprint: i32::from(args.sprint),
        sprint_name: &sprint_name,
        rows: &build.rows,
        note_text: None,
        approval_comment_url: None,
        issue_body_text: issue_body_for_comment.as_deref(),
    })
    .map_err(|err| CommandError::runtime("render-sprint-comment-failed", err))?;

    let comment_out = render::default_sprint_comment_path(
        &args.plan,
        i32::from(args.sprint),
        SprintCommentMode::Start,
    );
    render::write_rendered(&comment_out, &comment)
        .map_err(|err| CommandError::runtime("comment-write-failed", err))?;

    let should_comment = should_emit_comment(&args.comment_mode);
    if binary == BinaryFlavor::PlanIssue && should_comment && !dry_run {
        let repo = resolve_repo_for_live(binary, repo_override)?;
        adapter
            .comment_issue(&repo, args.issue, &comment_out)
            .map_err(|err| CommandError::runtime("github-comment-failed", err))?;
        live_mutations = true;
    }

    Ok(json!({
        "scope": "sprint",
        "sprint": args.sprint,
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "task_spec_path": path_text(&task_spec_out),
        "comment_path": path_text(&comment_out),
        "record_count": build.rows.len(),
        "subagent_prompts_out": path_text(&prompts_out),
        "subagent_prompt_files": prompt_files,
        "synced_issue_rows": synced_rows,
        "comment_requested": should_comment,
        "live_mutations_performed": live_mutations,
    }))
}

fn run_ready_sprint(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &ReadySprintArgs,
) -> Result<Value, CommandError> {
    let options = to_build_options(
        args.prefixes.owner_prefix.clone(),
        args.prefixes.branch_prefix.clone(),
        args.prefixes.worktree_prefix.clone(),
        args.grouping.pr_grouping,
        args.grouping.strategy,
        args.grouping.pr_group.clone(),
    );

    let build = task_spec::build_task_spec(
        &args.plan,
        TaskSpecScope::Sprint(i32::from(args.sprint)),
        &options,
    )
    .map_err(|err| CommandError::runtime("task-spec-generation-failed", err))?;

    let task_spec_out = args.task_spec_out.clone().unwrap_or_else(|| {
        task_spec::default_sprint_task_spec_path(&args.plan, i32::from(args.sprint))
    });
    task_spec::write_tsv(&task_spec_out, &build.rows)
        .map_err(|err| CommandError::runtime("task-spec-write-failed", err))?;

    let summary = load_summary(&args.summary)?;
    let sprint_name = build
        .sprint_name
        .clone()
        .unwrap_or_else(|| format!("Sprint {}", args.sprint));

    let adapter = GhCliAdapter;
    let mut issue_body_for_comment: Option<String> = None;
    let mut live_mutations = false;

    if binary == BinaryFlavor::PlanIssue {
        let repo = resolve_repo_for_live(binary, repo_override)?;
        let body = adapter
            .issue_body(&repo, args.issue)
            .map_err(|err| CommandError::runtime("github-issue-read-failed", err))?;
        let table = issue_body::parse_task_table(&body)
            .map_err(|err| CommandError::runtime("issue-body-parse-failed", err))?;
        let structure_errors = issue_body::validate_rows(table.rows());
        if !structure_errors.is_empty() {
            return Err(CommandError::runtime(
                "issue-body-invalid",
                structure_errors.join(" | "),
            ));
        }
        issue_body_for_comment = Some(body);
    }

    let comment = render::render_sprint_comment(SprintCommentInput {
        mode: SprintCommentMode::Ready,
        plan_file: &args.plan,
        sprint: i32::from(args.sprint),
        sprint_name: &sprint_name,
        rows: &build.rows,
        note_text: summary.as_deref(),
        approval_comment_url: None,
        issue_body_text: issue_body_for_comment.as_deref(),
    })
    .map_err(|err| CommandError::runtime("render-sprint-comment-failed", err))?;

    let comment_out = render::default_sprint_comment_path(
        &args.plan,
        i32::from(args.sprint),
        SprintCommentMode::Ready,
    );
    render::write_rendered(&comment_out, &comment)
        .map_err(|err| CommandError::runtime("comment-write-failed", err))?;

    let should_comment = should_emit_comment(&args.comment_mode);
    if binary == BinaryFlavor::PlanIssue && should_comment && !dry_run {
        let repo = resolve_repo_for_live(binary, repo_override)?;
        adapter
            .comment_issue(&repo, args.issue, &comment_out)
            .map_err(|err| CommandError::runtime("github-comment-failed", err))?;
        live_mutations = true;
    }

    Ok(json!({
        "scope": "sprint",
        "sprint": args.sprint,
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "task_spec_path": path_text(&task_spec_out),
        "comment_path": path_text(&comment_out),
        "record_count": build.rows.len(),
        "comment_requested": should_comment,
        "live_mutations_performed": live_mutations,
    }))
}

fn run_accept_sprint(
    binary: BinaryFlavor,
    dry_run: bool,
    repo_override: Option<&str>,
    args: &AcceptSprintArgs,
) -> Result<Value, CommandError> {
    if !approval_comment_url_looks_valid(&args.approved_comment_url) {
        return Err(CommandError::usage(
            "invalid-approval-comment-url",
            "--approved-comment-url must be a GitHub issue/pull comment URL",
        ));
    }

    let options = to_build_options(
        args.prefixes.owner_prefix.clone(),
        args.prefixes.branch_prefix.clone(),
        args.prefixes.worktree_prefix.clone(),
        args.grouping.pr_grouping,
        args.grouping.strategy,
        args.grouping.pr_group.clone(),
    );

    let build = task_spec::build_task_spec(
        &args.plan,
        TaskSpecScope::Sprint(i32::from(args.sprint)),
        &options,
    )
    .map_err(|err| CommandError::runtime("task-spec-generation-failed", err))?;

    let task_spec_out = args.task_spec_out.clone().unwrap_or_else(|| {
        task_spec::default_sprint_task_spec_path(&args.plan, i32::from(args.sprint))
    });
    task_spec::write_tsv(&task_spec_out, &build.rows)
        .map_err(|err| CommandError::runtime("task-spec-write-failed", err))?;

    let summary = load_summary(&args.summary)?;
    let sprint_name = build
        .sprint_name
        .clone()
        .unwrap_or_else(|| format!("Sprint {}", args.sprint));

    let adapter = GhCliAdapter;
    let mut issue_body_for_comment: Option<String> = None;
    let mut synced_done_rows = 0usize;
    let mut live_mutations = false;

    if binary == BinaryFlavor::PlanIssue {
        let repo = resolve_repo_for_live(binary, repo_override)?;
        let body = adapter
            .issue_body(&repo, args.issue)
            .map_err(|err| CommandError::runtime("github-issue-read-failed", err))?;

        let mut table = issue_body::parse_task_table(&body)
            .map_err(|err| CommandError::runtime("issue-body-parse-failed", err))?;
        let structure_errors = issue_body::validate_rows(table.rows());
        if !structure_errors.is_empty() {
            return Err(CommandError::runtime(
                "issue-body-invalid",
                structure_errors.join(" | "),
            ));
        }

        let sprint_indexes = table.sprint_row_indexes(i32::from(args.sprint));
        if sprint_indexes.is_empty() {
            return Err(CommandError::runtime(
                "sprint-not-found",
                format!("issue task table has no rows for sprint {}", args.sprint),
            ));
        }

        let sprint_rows: Vec<TaskRow> = sprint_indexes
            .iter()
            .map(|idx| table.rows()[*idx].clone())
            .collect();

        let required_prs = collect_required_prs(&sprint_rows, "accept-sprint")
            .map_err(|err| CommandError::runtime("sprint-acceptance-gate-failed", err))?;
        ensure_prs_merged(&adapter, &repo, &required_prs, "accept-sprint")
            .map_err(|err| CommandError::runtime("sprint-acceptance-gate-failed", err))?;

        for idx in sprint_indexes {
            let row = &mut table.rows_mut()[idx];
            row.status = "done".to_string();
            row.pr = issue_body::normalize_pr_display(&row.pr);
            synced_done_rows += 1;
        }

        let updated_body = table.render();
        issue_body_for_comment = Some(updated_body.clone());

        if !dry_run {
            let body_path = write_temp_markdown("accept-sprint-issue-body", &updated_body)
                .map_err(|err| CommandError::runtime("issue-body-write-failed", err))?;
            adapter
                .edit_issue_body(&repo, args.issue, &body_path)
                .map_err(|err| CommandError::runtime("github-issue-update-failed", err))?;
            live_mutations = true;
        }
    }

    let comment = render::render_sprint_comment(SprintCommentInput {
        mode: SprintCommentMode::Accepted,
        plan_file: &args.plan,
        sprint: i32::from(args.sprint),
        sprint_name: &sprint_name,
        rows: &build.rows,
        note_text: summary.as_deref(),
        approval_comment_url: Some(&args.approved_comment_url),
        issue_body_text: issue_body_for_comment.as_deref(),
    })
    .map_err(|err| CommandError::runtime("render-sprint-comment-failed", err))?;

    let comment_out = render::default_sprint_comment_path(
        &args.plan,
        i32::from(args.sprint),
        SprintCommentMode::Accepted,
    );
    render::write_rendered(&comment_out, &comment)
        .map_err(|err| CommandError::runtime("comment-write-failed", err))?;

    let should_comment = should_emit_comment(&args.comment_mode);
    if binary == BinaryFlavor::PlanIssue && should_comment && !dry_run {
        let repo = resolve_repo_for_live(binary, repo_override)?;
        adapter
            .comment_issue(&repo, args.issue, &comment_out)
            .map_err(|err| CommandError::runtime("github-comment-failed", err))?;
        live_mutations = true;
    }

    Ok(json!({
        "scope": "sprint",
        "sprint": args.sprint,
        "execution_mode": binary.execution_mode(),
        "dry_run": dry_run,
        "task_spec_path": path_text(&task_spec_out),
        "comment_path": path_text(&comment_out),
        "record_count": build.rows.len(),
        "approval_comment_url": args.approved_comment_url,
        "synced_done_rows": synced_done_rows,
        "comment_requested": should_comment,
        "live_mutations_performed": live_mutations,
    }))
}

fn run_multi_sprint_guide(args: &MultiSprintGuideArgs) -> Result<Value, CommandError> {
    let display_path = args.plan.to_string_lossy().to_string();
    let resolved_plan_path = task_spec::resolve_plan_file(&args.plan);
    if !resolved_plan_path.is_file() {
        return Err(CommandError::runtime(
            "plan-parse-failed",
            format!("plan file not found: {display_path}"),
        ));
    }
    let (plan, parse_errors) = parse_plan_with_display(&resolved_plan_path, &display_path)
        .map_err(|err| CommandError::runtime("plan-parse-failed", err.to_string()))?;

    if !parse_errors.is_empty() {
        return Err(CommandError::runtime(
            "plan-parse-failed",
            parse_errors.join(" | "),
        ));
    }

    let from_sprint = i32::from(args.from_sprint);
    let max_sprint = plan
        .sprints
        .iter()
        .map(|s| s.number)
        .max()
        .unwrap_or(from_sprint);
    let to_sprint = args.to_sprint.map(i32::from).unwrap_or(max_sprint);

    if to_sprint < from_sprint {
        return Err(CommandError::usage(
            "invalid-sprint-range",
            "--to-sprint must be greater than or equal to --from-sprint",
        ));
    }

    let issue_body_path = render::default_plan_issue_body_path(&args.plan);
    let script = "$AGENT_HOME/skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh";

    let mut lines = vec![
        "MULTI_SPRINT_GUIDE_BEGIN".to_string(),
        "DESIGN=ONE_PLAN_ONE_ISSUE".to_string(),
        "MODE=DRY_RUN_LOCAL".to_string(),
        format!("PLAN_FILE={display_path}"),
        format!("PLAN_TITLE={}", plan.title),
        format!("FROM_SPRINT={from_sprint}"),
        format!("TO_SPRINT={to_sprint}"),
        "DRY_RUN_PLAN_ISSUE=DRY_RUN_PLAN_ISSUE".to_string(),
        format!("DRY_RUN_ISSUE_BODY={}", issue_body_path.display()),
    ];

    let mut step = 1usize;
    lines.push(format!(
        "STEP_{step}={script} start-plan --plan {display_path} --pr-grouping <per-sprint\\|group> --dry-run"
    ));
    step += 1;

    for sprint in from_sprint..=to_sprint {
        lines.push(format!(
            "STEP_{step}={script} start-sprint --plan {display_path} --issue DRY_RUN_PLAN_ISSUE --sprint {sprint} --pr-grouping <per-sprint\\|group> --no-comment --dry-run"
        ));
        step += 1;

        if sprint < to_sprint {
            lines.push(format!(
                "STEP_{step}={script} accept-sprint --plan {display_path} --issue DRY_RUN_PLAN_ISSUE --sprint {sprint} --approved-comment-url <approval-comment-url-sprint-{sprint}> --pr-grouping <per-sprint\\|group> --no-comment --dry-run"
            ));
            step += 1;
        }
    }

    lines.push(format!(
        "STEP_{step}={script} ready-plan --body-file {} --summary Final\\ plan\\ review --no-comment --no-label-update --dry-run",
        issue_body_path.display()
    ));
    step += 1;

    lines.push(format!(
        "STEP_{step}={script} close-plan --body-file {} --approved-comment-url <final-plan-approval-comment-url> --dry-run",
        issue_body_path.display()
    ));

    lines.extend([
        "NOTE_DRY_RUN=Dry-run guide is local-only and does not call GitHub.".to_string(),
        "NOTE_GROUP_MODE_DETERMINISTIC=When using --pr-grouping group with --strategy deterministic, pass --pr-group for every task in the selected scope.".to_string(),
        "NOTE_GROUP_MODE_AUTO=When using --pr-grouping group with --strategy auto, --pr-group mappings are optional pins and remaining tasks are auto-grouped.".to_string(),
        "NOTE_SPRINT_GATE=Before starting sprint N+1, sprint N must be reviewed, merged, and accepted.".to_string(),
        "NOTE_ACCEPT_SYNC=accept-sprint enforces merged PRs for the sprint and syncs sprint task Status to done.".to_string(),
        "MULTI_SPRINT_GUIDE_END".to_string(),
    ]);

    Ok(json!({
        "scope": "plan",
        "from_sprint": from_sprint,
        "to_sprint": to_sprint,
        "guide": lines.join("\n"),
    }))
}

fn to_build_options(
    owner_prefix: String,
    branch_prefix: String,
    worktree_prefix: String,
    pr_grouping: crate::commands::PrGrouping,
    strategy: crate::commands::SplitStrategy,
    pr_group: Vec<crate::commands::PrGroupMapping>,
) -> TaskSpecBuildOptions {
    TaskSpecBuildOptions {
        owner_prefix,
        branch_prefix,
        worktree_prefix,
        pr_grouping,
        strategy,
        pr_group,
    }
}

fn load_summary(summary: &SummaryArgs) -> Result<Option<String>, CommandError> {
    if let Some(inline) = &summary.summary {
        return Ok(Some(inline.to_string()));
    }
    if let Some(path) = &summary.summary_file {
        let text = fs::read_to_string(path).map_err(|err| {
            CommandError::runtime(
                "summary-read-failed",
                format!("failed to read summary file {}: {err}", path.display()),
            )
        })?;
        return Ok(Some(text));
    }
    Ok(None)
}

fn load_close_comment(
    comment: &crate::commands::CommentTextArgs,
) -> Result<Option<String>, CommandError> {
    if let Some(inline) = &comment.comment {
        return Ok(Some(inline.to_string()));
    }
    if let Some(path) = &comment.comment_file {
        let text = fs::read_to_string(path).map_err(|err| {
            CommandError::runtime(
                "close-comment-read-failed",
                format!(
                    "failed to read close comment file {}: {err}",
                    path.display()
                ),
            )
        })?;
        return Ok(Some(text));
    }

    Ok(None)
}

fn approval_comment_url_looks_valid(url: &str) -> bool {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://github.com/") {
        return false;
    }
    let Some((base, suffix)) = trimmed.split_once("#issuecomment-") else {
        return false;
    };
    if !suffix.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    base.contains("/issues/") || base.contains("/pull/")
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn ensure_live_binary(binary: BinaryFlavor) -> Result<(), CommandError> {
    if binary == BinaryFlavor::PlanIssue {
        Ok(())
    } else {
        Err(CommandError::usage(
            "live-command-unavailable",
            "this command path requires the live `plan-issue` binary",
        ))
    }
}

fn resolve_repo_for_live(
    binary: BinaryFlavor,
    repo_override: Option<&str>,
) -> Result<String, CommandError> {
    ensure_live_binary(binary)?;
    crate::github::resolve_repo(repo_override)
        .map_err(|err| CommandError::usage("repo-resolution-failed", err))
}

fn render_plan_status_comment(rows: &[TaskRow]) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let status = row.status.trim().to_ascii_lowercase();
        *counts.entry(status).or_insert(0) += 1;
    }

    format!(
        "## Plan Status Snapshot\n\n- Total tasks: {}\n- planned: {}\n- in-progress: {}\n- blocked: {}\n- done: {}\n",
        rows.len(),
        counts.get("planned").copied().unwrap_or(0),
        counts.get("in-progress").copied().unwrap_or(0),
        counts.get("blocked").copied().unwrap_or(0),
        counts.get("done").copied().unwrap_or(0),
    )
}

fn should_emit_comment(comment_mode: &crate::commands::CommentModeArgs) -> bool {
    !comment_mode.no_comment
}

fn write_temp_markdown(stem: &str, content: &str) -> Result<PathBuf, String> {
    let dir = task_spec::agent_home()
        .join("out")
        .join("plan-issue-delivery-loop")
        .join("tmp");
    fs::create_dir_all(&dir).map_err(|err| {
        format!(
            "failed to create temp output directory {}: {err}",
            dir.display()
        )
    })?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("failed to compute timestamp: {err}"))?
        .as_millis();
    let path = dir.join(format!("{stem}-{now}.md"));
    fs::write(&path, content).map_err(|err| {
        format!(
            "failed to write temporary markdown {}: {err}",
            path.display()
        )
    })?;
    Ok(path)
}

fn default_subagent_prompts_path(plan_file: &Path, sprint: i32) -> PathBuf {
    let plan_stem = plan_file
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("plan")
        .to_string();

    task_spec::agent_home()
        .join("out")
        .join("plan-issue-delivery-loop")
        .join(format!("{plan_stem}-sprint-{sprint}-subagent-prompts"))
}

fn write_subagent_prompts(
    out_dir: &Path,
    issue: u64,
    sprint: i32,
    rows: &[TaskSpecRow],
) -> Result<Vec<String>, String> {
    fs::create_dir_all(out_dir).map_err(|err| {
        format!(
            "failed to create subagent prompt dir {}: {err}",
            out_dir.display()
        )
    })?;

    let mut paths = Vec::new();
    for row in rows {
        let path = out_dir.join(format!("{}-subagent-prompt.md", row.task_id));
        let body = format!(
            "# Subagent Task Prompt\n\n- Issue: #{issue}\n- Sprint: S{sprint}\n- Task: {}\n- Summary: {}\n- Owner: {}\n- Branch: {}\n- Worktree: {}\n- Notes: {}\n",
            row.task_id, row.summary, row.owner, row.branch, row.worktree, row.notes
        );
        fs::write(&path, body)
            .map_err(|err| format!("failed to write subagent prompt {}: {err}", path.display()))?;
        paths.push(path.to_string_lossy().to_string());
    }

    Ok(paths)
}

fn collect_required_prs(rows: &[TaskRow], scope: &str) -> Result<Vec<u64>, String> {
    let mut errors = Vec::new();
    let mut prs = Vec::new();
    let mut seen = HashSet::new();

    for row in rows {
        match issue_body::parse_pr_number(&row.pr) {
            Some(number) => {
                if seen.insert(number) {
                    prs.push(number);
                }
            }
            None => errors.push(format!(
                "{}: {} requires concrete PR reference (found `{}`)",
                row.task,
                scope,
                row.pr.trim()
            )),
        }
    }

    if !errors.is_empty() {
        return Err(errors.join(" | "));
    }

    Ok(prs)
}

fn ensure_prs_merged(
    adapter: &impl GitHubAdapter,
    repo: &str,
    prs: &[u64],
    scope: &str,
) -> Result<(), String> {
    let mut errors = Vec::new();

    for pr in prs {
        let merged = adapter
            .pr_is_merged(repo, *pr)
            .map_err(|err| format!("failed to query PR #{pr}: {err}"))?;
        if !merged {
            errors.push(format!("{scope}: PR #{pr} is not merged"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" | "))
    }
}

fn enforce_previous_sprint_gate(
    adapter: &impl GitHubAdapter,
    repo: &str,
    rows: &[TaskRow],
    sprint: i32,
) -> Result<(), String> {
    let previous = sprint - 1;
    let prev_rows: Vec<TaskRow> = rows
        .iter()
        .filter(|row| issue_body::row_sprint(row) == Some(previous))
        .cloned()
        .collect();

    if prev_rows.is_empty() {
        return Err(format!(
            "start-sprint gate: no rows found for previous sprint S{previous}"
        ));
    }

    let mut errors = Vec::new();
    for row in &prev_rows {
        if !row.status.trim().eq_ignore_ascii_case("done") {
            errors.push(format!(
                "{}: previous sprint gate requires Status=done (found `{}`)",
                row.task,
                row.status.trim()
            ));
        }
    }

    let prs = prev_rows
        .iter()
        .map(|row| {
            issue_body::parse_pr_number(&row.pr).ok_or_else(|| {
                format!(
                    "{}: previous sprint gate requires concrete PR reference (found `{}`)",
                    row.task,
                    row.pr.trim()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut unique_prs = prs;
    unique_prs.sort_unstable();
    unique_prs.dedup();

    if let Err(err) = ensure_prs_merged(adapter, repo, &unique_prs, "previous-sprint-gate") {
        errors.push(err);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" | "))
    }
}

fn sync_issue_rows_from_task_spec(
    table: &mut issue_body::TaskTable,
    spec_rows: &[TaskSpecRow],
) -> Result<usize, String> {
    let mut group_sizes: HashMap<String, usize> = HashMap::new();
    for spec in spec_rows {
        *group_sizes.entry(spec.pr_group.clone()).or_insert(0) += 1;
    }

    let mut spec_by_task: HashMap<String, (TaskSpecRow, String)> = HashMap::new();
    for spec in spec_rows {
        let mode = if spec.grouping == crate::commands::PrGrouping::PerSprint {
            "per-sprint".to_string()
        } else if group_sizes.get(&spec.pr_group).copied().unwrap_or(0) > 1 {
            "pr-shared".to_string()
        } else {
            "pr-isolated".to_string()
        };
        spec_by_task.insert(spec.task_id.clone(), (spec.clone(), mode));
    }

    let mut touched = HashSet::new();
    let mut updated = 0usize;

    for row in table.rows_mut() {
        if let Some((spec, mode)) = spec_by_task.get(&row.task) {
            row.summary = spec.summary.clone();
            row.owner = spec.owner.clone();
            row.branch = spec.branch.clone();
            row.worktree = spec.worktree.clone();
            row.execution_mode = mode.clone();
            row.notes = spec.notes.clone();
            if issue_body::is_placeholder(&row.pr) {
                row.pr = "TBD".to_string();
            } else {
                row.pr = issue_body::normalize_pr_display(&row.pr);
            }
            if row.status.trim().is_empty() {
                row.status = "planned".to_string();
            }
            touched.insert(spec.task_id.clone());
            updated += 1;
        }
    }

    if touched.len() != spec_rows.len() {
        let missing = spec_rows
            .iter()
            .filter(|row| !touched.contains(&row.task_id))
            .map(|row| row.task_id.clone())
            .collect::<Vec<_>>();
        return Err(format!(
            "issue task table missing rows for sprint tasks: {}",
            missing.join(",")
        ));
    }

    Ok(updated)
}

#[derive(Debug, Default)]
struct CleanupOutcome {
    targeted: Vec<String>,
    removed: Vec<String>,
    residual: Vec<String>,
}

#[derive(Debug, Clone)]
struct LinkedWorktree {
    path: PathBuf,
    branch: Option<String>,
}

fn cleanup_worktrees_from_rows(rows: &[TaskRow], dry_run: bool) -> Result<CleanupOutcome, String> {
    let repo_root = repo_root()?;
    let cwd = std::env::current_dir()
        .map_err(|err| format!("failed to read current directory: {err}"))?;

    let mut branch_targets: HashSet<String> = HashSet::new();
    let mut path_targets: HashSet<String> = HashSet::new();

    for row in rows {
        if !issue_body::is_placeholder(&row.branch) {
            branch_targets.insert(normalize_branch_name(&row.branch));
        }
        if !issue_body::is_placeholder(&row.worktree) {
            let resolved = resolve_worktree_path(&repo_root, row.worktree.trim());
            path_targets.insert(path_key(&resolved));
        }
    }

    let linked = list_linked_worktrees()?;
    let repo_root_key = path_key(&repo_root);

    let mut outcome = CleanupOutcome::default();

    for worktree in linked {
        let worktree_key = path_key(&worktree.path);
        let branch_key = worktree.branch.as_ref().map(|b| normalize_branch_name(b));

        let targeted = path_targets.contains(&worktree_key)
            || branch_key
                .as_ref()
                .is_some_and(|branch| branch_targets.contains(branch));

        if !targeted {
            continue;
        }

        outcome
            .targeted
            .push(worktree.path.to_string_lossy().to_string());

        if worktree_key == repo_root_key {
            continue;
        }

        if cwd == worktree.path || cwd.starts_with(&worktree.path) {
            outcome
                .residual
                .push(worktree.path.to_string_lossy().to_string());
            continue;
        }

        if dry_run {
            outcome
                .removed
                .push(worktree.path.to_string_lossy().to_string());
            continue;
        }

        let status = ProcessCommand::new("git")
            .args([
                "worktree",
                "remove",
                "--force",
                worktree.path.to_string_lossy().as_ref(),
            ])
            .status()
            .map_err(|err| format!("failed to execute `git worktree remove`: {err}"))?;

        if status.success() {
            outcome
                .removed
                .push(worktree.path.to_string_lossy().to_string());
        } else {
            outcome
                .residual
                .push(worktree.path.to_string_lossy().to_string());
        }
    }

    if !dry_run {
        let prune_status = ProcessCommand::new("git")
            .args(["worktree", "prune"])
            .status()
            .map_err(|err| format!("failed to execute `git worktree prune`: {err}"))?;
        if !prune_status.success() {
            return Err("git worktree prune failed".to_string());
        }

        let remaining = list_linked_worktrees()?;
        for worktree in remaining {
            let worktree_key = path_key(&worktree.path);
            let branch_key = worktree.branch.as_ref().map(|b| normalize_branch_name(b));
            let targeted = path_targets.contains(&worktree_key)
                || branch_key
                    .as_ref()
                    .is_some_and(|branch| branch_targets.contains(branch));

            if targeted && worktree_key != repo_root_key {
                let path = worktree.path.to_string_lossy().to_string();
                if !outcome.residual.contains(&path) {
                    outcome.residual.push(path);
                }
            }
        }

        if !outcome.residual.is_empty() {
            return Err(format!(
                "cleanup left targeted residual worktrees: {}",
                outcome.residual.join(", ")
            ));
        }
    }

    outcome.targeted.sort();
    outcome.targeted.dedup();
    outcome.removed.sort();
    outcome.removed.dedup();

    Ok(outcome)
}

fn list_linked_worktrees() -> Result<Vec<LinkedWorktree>, String> {
    let output = ProcessCommand::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|err| format!("failed to run `git worktree list --porcelain`: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "`git worktree list --porcelain` failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut rows = Vec::new();
    let mut current: Option<LinkedWorktree> = None;

    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(prev) = current.take() {
                rows.push(prev);
            }
            current = Some(LinkedWorktree {
                path: PathBuf::from(path.trim()),
                branch: None,
            });
            continue;
        }

        if let Some(branch) = line.strip_prefix("branch ")
            && let Some(current) = current.as_mut()
        {
            current.branch = Some(branch.trim().trim_start_matches("refs/heads/").to_string());
            continue;
        }

        if line.trim().is_empty()
            && let Some(prev) = current.take()
        {
            rows.push(prev);
        }
    }

    if let Some(prev) = current {
        rows.push(prev);
    }

    Ok(rows)
}

fn repo_root() -> Result<PathBuf, String> {
    let output = ProcessCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|err| format!("failed to run `git rev-parse --show-toplevel`: {err}"))?;

    if !output.status.success() {
        return Err("unable to resolve repository root".to_string());
    }

    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn resolve_worktree_path(repo_root: &Path, worktree: &str) -> PathBuf {
    let path = PathBuf::from(worktree);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn normalize_branch_name(branch: &str) -> String {
    branch.trim().trim_start_matches("refs/heads/").to_string()
}

fn path_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}
