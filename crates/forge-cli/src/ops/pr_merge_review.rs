//! Explicit merge review opt-out permits an absent observation, while existing
//! durable review state continues through the complete merge gate.

use crate::backend::BackendRunner;
use crate::error::ForgeError;
use crate::ops::{pr_comments::github_repo_slug_from_url, pr_review, pr_review_loop, review_state};
use crate::provider::ProviderContext;

pub(super) enum MergeReviewGate {
    Observed(pr_review_loop::ReviewLoopMergeGate),
    Unobserved { state_tip_digest: Option<String> },
}

impl MergeReviewGate {
    pub(super) fn into_observation(self) -> Option<pr_review_loop::ReviewLoopMergeGate> {
        match self {
            Self::Observed(gate) => Some(gate),
            Self::Unobserved { .. } => None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn ensure_merge_ready<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    pr_url: &str,
    number: u64,
    expected_head: &str,
    require_ledger: bool,
    explicit_convergence: Option<bool>,
) -> Result<Option<MergeReviewGate>, ForgeError> {
    if explicit_convergence == Some(false) {
        let repository = github_repo_slug_from_url(pr_url).ok_or_else(|| {
            ForgeError::software(
                "cli.forge-cli.pr.merge.v1",
                "unable to derive GitHub owner/repo from pull-request URL",
                Some(format!("url={pr_url}")),
            )
        })?;
        // Validate the complete trusted chain even when no observation exists.
        // An assignment alone must not require a ledger under an explicit
        // opt-out, as automated release PRs may have no review-loop workflow.
        let view = pr_review::read_review_loop_state_view(runner, ctx, &repository, number)?;
        if review_state::latest_review_loop_state(&view.chain).is_none() {
            return Ok(Some(MergeReviewGate::Unobserved {
                state_tip_digest: view.chain.tip_digest,
            }));
        }
    }
    // Preserve publication, head binding, hard stops, and blocking findings
    // whenever an observation exists; default and explicit-true are unchanged.
    // Explicit opt-out has already observed a ledger here, so a second read
    // cannot downgrade it to optional absence if the observation disappears.
    let require_ledger = require_ledger || explicit_convergence == Some(false);
    pr_review_loop::ensure_merge_ready(runner, ctx, pr_url, number, expected_head, require_ledger)
        .map(|gate| gate.map(MergeReviewGate::Observed))
}

/// Retain the initial trusted-chain tip even when opt-out permits no observation.
pub(super) fn recheck_merge_ready<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    pr_url: &str,
    number: u64,
    expected_head: &str,
    previous: &MergeReviewGate,
) -> Result<MergeReviewGate, ForgeError> {
    match previous {
        MergeReviewGate::Observed(gate) => {
            pr_review_loop::recheck_merge_ready(runner, ctx, pr_url, number, expected_head, gate)
                .map(MergeReviewGate::Observed)
        }
        MergeReviewGate::Unobserved { state_tip_digest } => {
            let repository = github_repo_slug_from_url(pr_url).ok_or_else(|| {
                ForgeError::software(
                    "cli.forge-cli.pr.merge.v1",
                    "unable to derive GitHub owner/repo from pull-request URL",
                    Some(format!("url={pr_url}")),
                )
            })?;
            let view = pr_review::read_review_loop_state_view(runner, ctx, &repository, number)?;
            if view.chain.tip_digest != *state_tip_digest
                || review_state::latest_review_loop_state(&view.chain).is_some()
            {
                return Err(ForgeError::validation(
                    "cli.forge-cli.pr.merge.v1",
                    "review_state_conflict",
                    "the review chain changed after merge gates and before provider merge; \
                     re-run the merge to re-read the chain",
                    Some(format!(
                        "expected_tip={state_tip_digest:?}; provider_tip={:?}",
                        view.chain.tip_digest
                    )),
                ));
            }
            Ok(MergeReviewGate::Unobserved {
                state_tip_digest: view.chain.tip_digest,
            })
        }
    }
}
