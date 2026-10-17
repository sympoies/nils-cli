use clap::Args;

/// `pr review-dismiss` arguments: governed dismissal of one automated
/// CHANGES_REQUESTED review after a designated-review PASS at the same head.
#[derive(Args, Debug, Clone)]
#[command(after_help = "GUARDS\n  \
      The review must be CHANGES_REQUESTED, on --expected-head, and authored by a \
      provider App account whose login is listed in the user-global \
      [[review_convergence.bots]]; repository config never counts. A login alone \
      never makes an author automated. The PR must hold a designated-review PASS \
      at --expected-head submitted after the review. The designated reviewer's own \
      review is never dismissable, and a valid AGENT_SESSION_ID must show the actor \
      is neither the designated reviewer session nor its provider account. The \
      reason is posted as the dismissal message together with the head and actor. \
      --dry-run runs every guard and prints the dismissal plan.")]
pub struct PrReviewDismissArgs {
    /// Numeric pull request id.
    pub id: u64,
    /// Numeric provider review id (`database_id` in `pr reviews`).
    #[arg(long = "review-id", value_name = "ID", value_parser = clap::value_parser!(u64).range(1..))]
    pub review_id: u64,
    /// Exact current provider head SHA; the review must be on this head.
    #[arg(long = "expected-head", value_name = "SHA", value_parser = clap::builder::NonEmptyStringValueParser::new())]
    pub expected_head: String,
    /// Non-empty dismissal reason file (`-` reads stdin).
    #[arg(long = "reason-file", value_name = "PATH")]
    pub reason_file: String,
}
