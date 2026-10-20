use clap::Args;

/// Native review summaries, or one complete review selected by database ID.
#[derive(Args, Debug, Clone)]
pub struct PrReviewsArgs {
    /// Numeric pull request id.
    pub id: u64,
    /// Return one complete native review body instead of the summary listing.
    #[arg(long, value_name = "REVIEW_ID", value_parser = clap::value_parser!(u64).range(1..))]
    pub review_id: Option<u64>,
}
