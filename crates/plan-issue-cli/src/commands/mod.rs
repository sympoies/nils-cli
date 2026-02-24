pub mod build;
pub mod plan;
pub mod sprint;

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::Value;

use crate::ValidationError;

use self::build::{BuildPlanTaskSpecArgs, BuildTaskSpecArgs};
use self::plan::{
    CleanupWorktreesArgs, ClosePlanArgs, ReadyPlanArgs, StartPlanArgs, StatusPlanArgs,
};
use self::sprint::{AcceptSprintArgs, MultiSprintGuideArgs, ReadySprintArgs, StartSprintArgs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
pub enum PrGrouping {
    #[value(name = "per-sprint", alias = "per-spring")]
    PerSprint,
    #[value(name = "group")]
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrGroupMapping {
    pub task: String,
    pub group: String,
}

fn parse_pr_group_mapping(raw: &str) -> Result<PrGroupMapping, String> {
    let (task_raw, group_raw) = raw
        .split_once('=')
        .ok_or_else(|| "expected format <task>=<group>".to_string())?;

    let task = task_raw.trim();
    let group = group_raw.trim();

    if task.is_empty() {
        return Err("task key in --pr-group cannot be empty".to_string());
    }
    if group.is_empty() {
        return Err("group name in --pr-group cannot be empty".to_string());
    }

    Ok(PrGroupMapping {
        task: task.to_string(),
        group: group.to_string(),
    })
}

#[derive(Debug, Clone, Args, Serialize)]
pub struct PrefixArgs {
    /// Task owner prefix.
    #[arg(long, default_value = "subagent", value_name = "text")]
    pub owner_prefix: String,

    /// Branch prefix.
    #[arg(long, default_value = "issue", value_name = "text")]
    pub branch_prefix: String,

    /// Worktree prefix.
    #[arg(long, default_value = "issue__", value_name = "text")]
    pub worktree_prefix: String,
}

#[derive(Debug, Clone, Args, Serialize)]
pub struct GroupingArgs {
    /// PR grouping mode.
    #[arg(long, value_enum, value_name = "mode")]
    pub pr_grouping: PrGrouping,

    /// Explicit task->group mapping (`<task>=<group>`). Repeatable.
    #[arg(
        long = "pr-group",
        value_name = "task=group",
        value_parser = parse_pr_group_mapping
    )]
    pub pr_group: Vec<PrGroupMapping>,
}

#[derive(Debug, Clone, Args, Default, Serialize)]
pub struct SummaryArgs {
    /// Inline review summary text.
    #[arg(long, conflicts_with = "summary_file", value_name = "text")]
    pub summary: Option<String>,

    /// Path to markdown/text review summary.
    #[arg(long, conflicts_with = "summary", value_name = "path")]
    pub summary_file: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Args, Default, Serialize)]
pub struct CommentModeArgs {
    /// Emit comment output.
    #[arg(long, conflicts_with = "no_comment")]
    pub comment: bool,

    /// Disable comment output.
    #[arg(long = "no-comment", conflicts_with = "comment")]
    pub no_comment: bool,
}

#[derive(Debug, Clone, Args, Default, Serialize)]
pub struct CommentTextArgs {
    /// Inline close comment.
    #[arg(long, conflicts_with = "comment_file", value_name = "text")]
    pub comment: Option<String>,

    /// Path to close comment markdown/text.
    #[arg(long, conflicts_with = "comment", value_name = "path")]
    pub comment_file: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Build sprint-scoped task-spec TSV from a plan.
    BuildTaskSpec(BuildTaskSpecArgs),

    /// Build plan-scoped task-spec TSV (all sprints) for the single plan issue.
    BuildPlanTaskSpec(BuildPlanTaskSpecArgs),

    /// Open one plan issue with all plan tasks in Task Decomposition.
    StartPlan(StartPlanArgs),

    /// Wrapper of issue-delivery-loop status for the plan issue.
    StatusPlan(StatusPlanArgs),

    /// Wrapper of issue-delivery-loop ready-for-review for final plan review.
    ReadyPlan(ReadyPlanArgs),

    /// Close the single plan issue after final approval and merged PR gates.
    ClosePlan(ClosePlanArgs),

    /// Enforce cleanup of all issue-assigned task worktrees.
    CleanupWorktrees(CleanupWorktreesArgs),

    /// Start sprint only after previous sprint merge+done gate passes.
    StartSprint(StartSprintArgs),

    /// Post sprint-ready comment for main-agent review before merge.
    ReadySprint(ReadySprintArgs),

    /// Enforce merged-PR gate, sync sprint status=done, then post accepted comment.
    AcceptSprint(AcceptSprintArgs),

    /// Print the full repeated command flow for a plan.
    MultiSprintGuide(MultiSprintGuideArgs),
}

impl Command {
    pub fn command_id(&self) -> &'static str {
        match self {
            Self::BuildTaskSpec(_) => "build-task-spec",
            Self::BuildPlanTaskSpec(_) => "build-plan-task-spec",
            Self::StartPlan(_) => "start-plan",
            Self::StatusPlan(_) => "status-plan",
            Self::ReadyPlan(_) => "ready-plan",
            Self::ClosePlan(_) => "close-plan",
            Self::CleanupWorktrees(_) => "cleanup-worktrees",
            Self::StartSprint(_) => "start-sprint",
            Self::ReadySprint(_) => "ready-sprint",
            Self::AcceptSprint(_) => "accept-sprint",
            Self::MultiSprintGuide(_) => "multi-sprint-guide",
        }
    }

    pub fn schema_version(&self) -> String {
        format!("plan-issue-cli.{}.v1", self.command_id().replace('-', "."))
    }

    pub fn payload(&self) -> Value {
        let payload = match self {
            Self::BuildTaskSpec(args) => serde_json::to_value(args),
            Self::BuildPlanTaskSpec(args) => serde_json::to_value(args),
            Self::StartPlan(args) => serde_json::to_value(args),
            Self::StatusPlan(args) => serde_json::to_value(args),
            Self::ReadyPlan(args) => serde_json::to_value(args),
            Self::ClosePlan(args) => serde_json::to_value(args),
            Self::CleanupWorktrees(args) => serde_json::to_value(args),
            Self::StartSprint(args) => serde_json::to_value(args),
            Self::ReadySprint(args) => serde_json::to_value(args),
            Self::AcceptSprint(args) => serde_json::to_value(args),
            Self::MultiSprintGuide(args) => serde_json::to_value(args),
        };

        payload.unwrap_or(Value::Null)
    }

    pub fn validate(&self, dry_run: bool) -> Result<(), ValidationError> {
        match self {
            Self::BuildTaskSpec(args) => validate_grouping(&args.grouping),
            Self::BuildPlanTaskSpec(args) => validate_grouping(&args.grouping),
            Self::StartPlan(args) => validate_grouping(&args.grouping),
            Self::StartSprint(args) => validate_grouping(&args.grouping),
            Self::ClosePlan(args) => validate_close_plan_args(args, dry_run),
            Self::StatusPlan(_)
            | Self::ReadyPlan(_)
            | Self::CleanupWorktrees(_)
            | Self::ReadySprint(_)
            | Self::AcceptSprint(_)
            | Self::MultiSprintGuide(_) => Ok(()),
        }
    }
}

fn validate_grouping(grouping: &GroupingArgs) -> Result<(), ValidationError> {
    match grouping.pr_grouping {
        PrGrouping::PerSprint if !grouping.pr_group.is_empty() => Err(ValidationError::new(
            "invalid-pr-grouping",
            "--pr-group is only valid when --pr-grouping group",
        )),
        PrGrouping::Group if grouping.pr_group.is_empty() => Err(ValidationError::new(
            "invalid-pr-grouping",
            "--pr-grouping group requires at least one --pr-group",
        )),
        _ => Ok(()),
    }
}

fn validate_close_plan_args(args: &ClosePlanArgs, dry_run: bool) -> Result<(), ValidationError> {
    if !dry_run && args.issue.is_none() {
        return Err(ValidationError::new(
            "missing-issue",
            "--issue is required for close-plan unless --dry-run is enabled",
        ));
    }

    if dry_run && args.issue.is_none() && args.body_file.is_none() {
        return Err(ValidationError::new(
            "missing-issue-source",
            "--dry-run close-plan requires --issue or --body-file",
        ));
    }

    Ok(())
}
