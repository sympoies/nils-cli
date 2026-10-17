//! Arguments for editing existing comments.
use super::CommentKind;
use clap::Args;

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
