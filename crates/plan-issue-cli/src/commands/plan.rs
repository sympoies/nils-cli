use std::path::PathBuf;

use clap::{ArgGroup, Args, ValueEnum};
use serde::Serialize;

use super::{CommentModeArgs, CommentTextArgs, GroupingArgs, PrefixArgs, SummaryArgs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
pub enum CloseReason {
    Completed,
    #[value(name = "not-planned")]
    NotPlanned,
}

#[derive(Debug, Clone, Args, Serialize)]
pub struct StartPlanArgs {
    /// Plan markdown path.
    #[arg(long, value_name = "path")]
    pub plan: PathBuf,

    /// Override plan issue title.
    #[arg(long, value_name = "text")]
    pub title: Option<String>,

    /// Plan task-spec output path override.
    #[arg(long, value_name = "path")]
    pub task_spec_out: Option<PathBuf>,

    /// Rendered plan issue body output path override.
    #[arg(long, value_name = "path")]
    pub issue_body_out: Option<PathBuf>,

    #[command(flatten)]
    pub prefixes: PrefixArgs,

    #[command(flatten)]
    pub grouping: GroupingArgs,

    /// Labels to add at issue creation time.
    #[arg(long = "label", value_name = "name", default_values = ["issue", "plan"])]
    pub label: Vec<String>,
}

#[derive(Debug, Clone, Args, Serialize)]
#[command(group(
    ArgGroup::new("issue_source")
        .required(true)
        .args(["issue", "body_file"])
))]
pub struct StatusPlanArgs {
    /// Plan issue number (live `plan-issue` path only).
    #[arg(long, value_name = "number")]
    pub issue: Option<u64>,

    /// Offline issue body path.
    #[arg(long, value_name = "path")]
    pub body_file: Option<PathBuf>,

    #[command(flatten)]
    pub comment_mode: CommentModeArgs,
}

#[derive(Debug, Clone, Args, Serialize)]
#[command(group(
    ArgGroup::new("issue_source")
        .required(true)
        .args(["issue", "body_file"])
))]
pub struct ReadyPlanArgs {
    /// Plan issue number (live `plan-issue` path only).
    #[arg(long, value_name = "number")]
    pub issue: Option<u64>,

    /// Offline issue body path.
    #[arg(long, value_name = "path")]
    pub body_file: Option<PathBuf>,

    #[command(flatten)]
    pub summary: SummaryArgs,

    /// Review label.
    #[arg(long = "label", value_name = "name", default_value = "needs-review")]
    pub label: String,

    /// Labels to remove.
    #[arg(long = "remove-label", value_name = "name")]
    pub remove_label: Vec<String>,

    #[command(flatten)]
    pub comment_mode: CommentModeArgs,

    /// Skip label updates.
    #[arg(long)]
    pub no_label_update: bool,
}

#[derive(Debug, Clone, Args, Serialize)]
pub struct ClosePlanArgs {
    /// Plan issue number (`--issue`-only path is live `plan-issue` only).
    #[arg(long, value_name = "number")]
    pub issue: Option<u64>,

    /// Local issue body path (dry-run mode).
    #[arg(long, value_name = "path")]
    pub body_file: Option<PathBuf>,

    /// Final approval comment URL.
    #[arg(long = "approved-comment-url", value_name = "url")]
    pub approved_comment_url: String,

    /// Close reason.
    #[arg(long, value_enum, default_value_t = CloseReason::Completed)]
    pub reason: CloseReason,

    #[command(flatten)]
    pub close_comment: CommentTextArgs,

    /// Allow closing when some tasks are not done.
    #[arg(long)]
    pub allow_not_done: bool,
}

#[derive(Debug, Clone, Args, Serialize)]
pub struct CleanupWorktreesArgs {
    /// Plan issue number (live `plan-issue` only).
    #[arg(long, value_name = "number")]
    pub issue: u64,
}
