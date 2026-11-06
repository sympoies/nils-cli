//! Arguments for editing existing comments.
use super::*;
use clap::{Args, Subcommand, ValueEnum};

#[derive(Args, Debug)]
pub struct CommentEditArgs {
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub id: u64,
    #[arg(long, value_enum, default_value = "issue")]
    pub kind: CommentKind,
    #[arg(
        long,
        required_unless_present = "body_file",
        conflicts_with = "body_file"
    )]
    pub body: Option<String>,
    /// Markdown body file; use - to read stdin.
    #[arg(long)]
    pub body_file: Option<String>,
    /// Refuse if updated_at differs. This is read-then-write, not atomic CAS;
    /// a concurrent edit after the preflight read can still be overwritten.
    #[arg(long)]
    pub expected_updated_at: Option<String>,
}

#[derive(Args, Debug)]
pub struct CommentArgs {
    #[command(subcommand)]
    pub command: CommentCommand,
}
#[derive(Subcommand, Debug)]
pub enum CommentCommand {
    /// Replace the body of an issue/PR timeline or review comment.
    Edit(CommentEditArgs),
    /// Replace one exact timeline comment as a repository maintainer.
    ///
    /// Replaces only the current body; edit history needs separate provider redaction.
    /// The digest check is read-then-write, not atomic CAS: concurrent edits can be overwritten.
    Redact(CommentRedactArgs),
    /// Delete an issue/PR timeline or review comment.
    Delete(CommentDeleteArgs),
}
#[derive(Clone, Copy, Debug, ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentKind {
    /// Issue and PR timeline comments.
    Issue,
    /// Pull request review (diff) comments.
    Review,
}
#[derive(Args, Debug)]
pub struct CommentDeleteArgs {
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub id: u64,
    #[arg(long, value_enum, default_value = "issue")]
    pub kind: CommentKind,
}

/// Replaces only the current body; edit history needs separate provider redaction.
/// The digest check is read-then-write, not atomic CAS: concurrent edits can be overwritten.
#[derive(Args, Debug)]
pub struct CommentRedactArgs {
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub id: u64,
    /// Inspect author and SHA-256 without returning the comment body or writing.
    #[arg(long, conflicts_with_all = ["expected_author", "expected_body_sha256", "body_file"])]
    pub inspect: bool,
    /// Expected original author login, verified before writing.
    #[arg(long, required_unless_present = "inspect")]
    pub expected_author: Option<String>,
    /// SHA-256 of the exact UTF-8 current body (64 lowercase hex digits).
    #[arg(long, required_unless_present = "inspect")]
    pub expected_body_sha256: Option<String>,
    /// Replacement Markdown file (stdin is not accepted).
    #[arg(long, required_unless_present = "inspect")]
    pub body_file: Option<String>,
}

/// `pr` subtree.
#[derive(Subcommand, Debug)]
pub enum PrCommand {
    /// Open a draft pull / merge request from the current branch.
    Create(PrCreateArgs),
    /// Fetch a single PR / MR with normalised fields.
    View {
        /// Numeric id or branch name.
        id: String,
    },
    /// List PRs / MRs.
    List(PrListArgs),
    /// Mutate PR / MR title, body, base, labels, reviewers.
    Edit(PrEditArgs),
    /// Append a comment to a PR / MR.
    Comment(PrCommentArgs),
    /// Edit an existing PR / MR comment.
    CommentEdit(TimelineCommentEditArgs),
    /// Post a review outcome comment and optionally mirror it to an issue.
    Review(PrReviewArgs),
    /// List the issue-style comment stream attached to a PR / MR.
    Comments(PrCommentsArgs),
    /// Promote a draft PR / MR to ready-for-review.
    Ready(PrReadyArgs),
    /// List review threads attached to a PR / MR with their resolved state.
    ReviewThreads(PrReviewThreadsArgs),
    /// List native review summaries, classified against the current head.
    Reviews(PrReviewsArgs),
    /// Recover an authenticated actor's pending native review.
    PendingReview(PrPendingReviewArgs),
    /// Inspect and advance the durable review-loop ledger.
    ReviewLoop(PrReviewLoopArgs),
    /// Explicit designated reviewer handoff and published-review admission.
    ReviewHandoff(PrReviewHandoffArgs),
    /// Dismiss one configured automated CHANGES_REQUESTED review after a designated PASS.
    ReviewDismiss(PrReviewDismissArgs),
    /// List GFM task-list items in the PR / MR description with their state.
    Tasks(PrTasksArgs),
    /// Merge a ready PR / MR.
    Merge(PrMergeArgs),
    /// Close a PR / MR without merging.
    Close {
        /// Numeric id.
        id: u64,
    },
    /// One-shot snapshot of PR / MR check state.
    Checks(PrChecksArgs),
    /// Block until every required check reaches a terminal state.
    WaitChecks(PrWaitChecksArgs),
    /// End-to-end "open draft (or adopt the branch's open PR) → CI green →
    /// ready → merge" macro.
    Deliver(PrDeliverArgs),
}

/// `issue` subtree.
#[derive(Subcommand, Debug)]
pub enum IssueCommand {
    /// Open a new issue.
    Create(IssueCreateArgs),
    /// Fetch a single issue.
    View(IssueViewArgs),
    /// List issues filtered by state / labels / author / assignee.
    List(IssueListArgs),
    /// Mutate an issue.
    Edit(IssueEditArgs),
    /// Append a comment to an issue.
    Comment(IssueCommentArgs),
    /// Edit an existing issue comment.
    CommentEdit(TimelineCommentEditArgs),
    /// Close an issue.
    Close(IssueCloseArgs),
    /// Reopen a closed issue.
    Reopen {
        /// Numeric id.
        id: u64,
    },
    /// Check and maintain a program tracker issue's phase table.
    Tracker(IssueTrackerArgs),
}

/// Edit a comment on the specified issue or PR.
#[derive(Args, Debug, Clone)]
pub struct TimelineCommentEditArgs {
    /// Parent issue or PR / MR number.
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub id: u64,
    /// Provider comment identifier.
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    pub comment_id: u64,
    /// Replacement body file; use - for stdin.
    #[arg(long, value_name = "PATH")]
    pub body_file: String,
}
