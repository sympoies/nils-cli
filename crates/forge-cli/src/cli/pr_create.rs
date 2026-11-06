use super::PrKindFlag;
use clap::{ArgAction, Args};

/// `pr create` arguments. Maps to the inputs declared in
/// `crates/forge-cli/docs/specs/forge-cli-ops-v1.yaml::operations.pr.create`.
#[derive(Args, Debug, Clone)]
pub struct PrCreateArgs {
    /// Source branch (defaults to the current branch).
    #[arg(long)]
    pub head: Option<String>,
    /// Target / base branch (defaults to the repo's default branch).
    #[arg(long)]
    pub base: Option<String>,
    /// PR / MR title (required).
    #[arg(long)]
    pub title: String,
    /// PR / MR body / description text. Mutually exclusive with `--body-file`.
    #[arg(long, conflicts_with = "body_file")]
    pub body: Option<String>,
    /// Read PR / MR body from a file. Use `-` to read from stdin.
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: Option<String>,
    /// PR / MR kind (selects branch-prefix rule). Required.
    #[arg(long, value_enum)]
    pub kind: PrKindFlag,
    /// Open as ready-for-review instead of draft (default: open as draft).
    #[arg(long = "no-draft", action = ArgAction::SetTrue)]
    pub no_draft: bool,
    /// Add a reviewer (repeatable).
    #[arg(long = "reviewer", value_name = "USER")]
    pub reviewers: Vec<String>,
    /// Add a label (repeatable).
    #[arg(long = "label", value_name = "LABEL")]
    pub labels: Vec<String>,
    /// Validate labels against this catalog when `--strict-labels` is set.
    #[arg(long = "label-catalog", value_name = "PATH")]
    pub label_catalog: Option<String>,
    /// Fail when selected labels are missing, not applicable, or conflict.
    #[arg(long = "strict-labels", action = ArgAction::SetTrue)]
    pub strict_labels: bool,
    /// Directory holding a `test-first-evidence` record. Verified by the
    /// test-first gate when `[test_first].require` is true and `require_kinds`
    /// selects this kind (default: feature/bug). Requires a complete v2 record
    /// with a testable classification, pre-fix evidence, scoped final validation, a
    /// residual-gap declaration, and a bound baseline/delivery subject
    /// matching the current checkout.
    #[arg(long = "test-first-evidence", value_name = "DIR")]
    pub test_first_evidence: Option<String>,
    /// Number of an earlier PR / MR this one supersedes. With
    /// `--test-first-evidence` pointing at that PR's record, the gate verifies the
    /// record against the old head and admits this head when it differs from the
    /// old head only by default-branch merges; the link is added to the body.
    #[arg(long, value_name = "PR", requires = "test_first_evidence")]
    pub supersedes: Option<u64>,
}
