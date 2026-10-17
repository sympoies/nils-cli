use clap::{ArgAction, Args};

/// `pr pending-review delete` arguments.
#[derive(Args, Debug, Clone)]
pub struct PrPendingReviewDeleteArgs {
    /// Numeric pull request id that must own the pending review.
    pub id: u64,
    /// Pending review node id (`PRR_...`) returned by `pr reviews`.
    #[arg(
        long,
        value_name = "REVIEW_ID",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub review: String,
    /// Pull-request head that must still be current immediately before deletion.
    #[arg(
        long = "expected-head",
        value_name = "SHA",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub expected_head: String,
    /// Commit to which the pending review must still be bound.
    #[arg(
        long = "expected-commit",
        value_name = "SHA",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub expected_commit: String,
    /// Exact pending-review body expected before deletion.
    #[arg(
        long = "expected-body",
        conflicts_with = "expected_body_file",
        required_unless_present = "expected_body_file"
    )]
    pub expected_body: Option<String>,
    /// Read the exact expected pending-review body from a file. Use `-` for stdin.
    #[arg(
        long = "expected-body-file",
        value_name = "PATH",
        conflicts_with = "expected_body",
        required_unless_present = "expected_body"
    )]
    pub expected_body_file: Option<String>,
    /// Confirm that the exact guarded pending review is an abandoned draft.
    #[arg(long = "confirm-abandoned", action = ArgAction::SetTrue, required = true)]
    pub confirm_abandoned: bool,
}
