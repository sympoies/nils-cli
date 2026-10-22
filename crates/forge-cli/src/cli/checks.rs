//! Clap arguments for the one-shot checks snapshot.

use clap::{ArgAction, Args};

/// `pr checks` arguments. Maps to
/// `forge-cli-ops-v1.yaml::operations.pr.checks` inputs.
#[derive(Args, Debug, Clone)]
pub struct PrChecksArgs {
    /// Numeric id or branch name (GitHub accepts both natively; GitLab
    /// resolves numeric ids via `mr view` to fetch the source branch).
    pub id: String,
    /// Restrict the gating decision to required checks (default `true`).
    /// Non-required checks are always reported in `data.checks` regardless.
    #[arg(
        long = "required-only",
        action = ArgAction::Set,
        default_value_t = true,
        num_args = 0..=1,
        default_missing_value = "true",
    )]
    pub required_only: bool,
    /// Require successful execution of every gating check. Skipped and neutral
    /// checks fail the gate instead of satisfying it; exits 1 for a non-passing
    /// gate. By default, skipped and neutral checks satisfy the gate.
    #[arg(long, action = ArgAction::SetTrue)]
    pub strict: bool,
}
