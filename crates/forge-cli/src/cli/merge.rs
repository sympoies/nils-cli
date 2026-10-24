use super::MergeMethodFlag;
use clap::{ArgAction, Args};

/// `pr merge` arguments. Maps to
/// `forge-cli-ops-v1.yaml::operations.pr.merge` inputs.
#[derive(Args, Debug, Clone)]
pub struct PrMergeArgs {
    /// Numeric PR / MR id.
    pub id: u64,
    /// Compare-and-swap head verified by the caller before merge.
    #[arg(
        long = "expected-head",
        value_name = "SHA",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub expected_head_sha: Option<String>,
    /// Exact target / base branch the provider PR/MR must still use.
    #[arg(
        long = "expected-base",
        value_name = "BRANCH",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub expected_base: Option<String>,
    /// Merge method override. When omitted, falls back to
    /// `.forge-cli.toml [merge].method` then the spec default `squash`.
    #[arg(long, value_enum)]
    pub method: Option<MergeMethodFlag>,
    /// Skip the post-merge branch deletion (`--delete-branch` on gh,
    /// `--remove-source-branch` on glab). Mutually exclusive with the
    /// implicit-true default; flagging both is `keep_branch_conflict`.
    #[arg(long = "keep-branch", action = ArgAction::SetTrue)]
    pub keep_branch: bool,
    /// Delete the source branch even when open PRs/MRs target it. May close dependents.
    #[arg(long, conflicts_with = "keep_branch")]
    pub delete_branch_even_if_stacked: bool,
    /// Allow merges where the PR's base is not the repo's default branch.
    /// Without this flag, mismatched bases trigger `default_branch_protected`.
    #[arg(long = "allow-non-default-base", action = ArgAction::SetTrue)]
    pub allow_non_default_base: bool,
    /// Merge despite unresolved review threads. Without this flag, any
    /// non-outdated unresolved thread (bot or human) triggers
    /// `unresolved_review_threads`. Requires `--allow-unresolved-threads-reason`.
    #[arg(
        long = "allow-unresolved-threads",
        action = ArgAction::SetTrue,
        requires = "allow_unresolved_threads_reason"
    )]
    pub allow_unresolved_threads: bool,
    /// Required when `--allow-unresolved-threads` is set. Non-empty free-form
    /// text describing why the unresolved threads are safe to merge past; the
    /// reason is recorded in the merge envelope payload.
    #[arg(
        long = "allow-unresolved-threads-reason",
        value_name = "TEXT",
        requires = "allow_unresolved_threads",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub allow_unresolved_threads_reason: Option<String>,
    /// Override `[review_convergence].require`. Passing the flag without a
    /// value enables it; use `--review-convergence=false` to disable a repo or
    /// user-global opt-in for this invocation.
    #[arg(
        long = "review-convergence",
        action = ArgAction::Set,
        num_args = 0..=1,
        default_missing_value = "true"
    )]
    pub review_convergence: Option<bool>,
    /// Merge a head for which no required checks are registered. Without this
    /// flag an empty required-check snapshot triggers `checks_not_registered`,
    /// because "all required checks passed" is vacuously true over an empty set
    /// and so proves nothing about the head. Requires
    /// `--allow-no-checks-reason`. `[checks] none = true` with a
    /// `none_reason` in the base branch's `.forge-cli.toml` has the same
    /// effect.
    #[arg(
        long = "allow-no-checks",
        action = ArgAction::SetTrue,
        requires = "allow_no_checks_reason"
    )]
    pub allow_no_checks: bool,
    /// Required when `--allow-no-checks` is set. Non-empty free-form text
    /// describing why merging an unchecked head is safe — normally that the
    /// repository configures no checks at all; the reason is recorded in the
    /// merge envelope payload.
    #[arg(
        long = "allow-no-checks-reason",
        value_name = "TEXT",
        requires = "allow_no_checks",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub allow_no_checks_reason: Option<String>,
    /// Merge despite unchecked task-list items in the PR/MR description.
    /// Without this flag, any unchecked `- [ ]` item triggers
    /// `unchecked_task_items`. Requires `--allow-unchecked-tasks-reason`.
    #[arg(
        long = "allow-unchecked-tasks",
        action = ArgAction::SetTrue,
        requires = "allow_unchecked_tasks_reason"
    )]
    pub allow_unchecked_tasks: bool,
    /// Required when `--allow-unchecked-tasks` is set. Non-empty free-form
    /// text describing why the unchecked items are safe to merge past; the
    /// reason is recorded in the merge envelope payload.
    #[arg(
        long = "allow-unchecked-tasks-reason",
        value_name = "TEXT",
        requires = "allow_unchecked_tasks",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub allow_unchecked_tasks_reason: Option<String>,
    /// Merge despite an active merge freeze (an open `merge-freeze` issue) the
    /// caller holds. Repeat to name every active freeze; one left unnamed
    /// still triggers `merge_freeze_active`. Requires
    /// `--allow-merge-freeze-reason`.
    #[arg(
        long = "allow-merge-freeze",
        value_name = "ISSUE",
        action = ArgAction::Append,
        requires = "allow_merge_freeze_reason"
    )]
    pub allow_merge_freeze: Vec<u64>,
    /// Required when `--allow-merge-freeze` is set; recorded in the merge
    /// envelope payload.
    #[arg(
        long = "allow-merge-freeze-reason",
        value_name = "TEXT",
        requires = "allow_merge_freeze",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub allow_merge_freeze_reason: Option<String>,
    /// Upper bound, in seconds, on waiting for a required merge queue to merge
    /// the enqueued PR (default 5700: 90 minutes plus 5 minutes polling margin).
    /// This flag overrides the built-in default; no environment/config setting.
    #[arg(long = "queue-timeout", value_name = "SECONDS")]
    pub queue_timeout: Option<u64>,
}
