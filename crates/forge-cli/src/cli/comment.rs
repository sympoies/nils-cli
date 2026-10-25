//! Arguments for editing existing comments.
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
