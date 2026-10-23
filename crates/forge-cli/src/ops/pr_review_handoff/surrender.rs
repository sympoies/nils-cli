//! Relinquishing the retained interval does not authorize a review write.
use super::{ReviewHandoff, actor_digest, assigned, fail, review_state};
use crate::error::ForgeError;

/// Surrender is bound to the exact retained ledger tip at both command and
/// append admission. Its generation comes from that record, not ambient setup.
/// Appending observations and publishing still require the generation explicitly.
pub(super) fn require_reviewer(
    chain: &review_state::ReviewStateChain,
) -> Result<Option<&ReviewHandoff>, ForgeError> {
    let handoff = assigned(chain)?;
    if let Some(handoff) = handoff
        && actor_digest()? != handoff.reviewer_digest
    {
        return Err(fail(
            "review_writer_conflict",
            "only the designated reviewer session may surrender, append or publish this review",
        ));
    }
    Ok(handoff)
}

pub(super) fn recovery(kind: &str, message: &str) -> &'static str {
    match kind {
        "review_assignment_generation_conflict" => {
            "recovery=run `forge-cli pr review-handoff inspect` to read the retained assignment_generation, then set AGENT_REVIEW_ASSIGNMENT_GENERATION for the persisted reviewer; to relinquish instead, run `forge-cli pr review-handoff surrender` with the exact --expected-head and --expected-state"
        }
        "review_handover_required" | "review_writer_conflict" => {
            if message.contains("repository-configured handoff supervisor") {
                "recovery=the original coordinator can run `forge-cli pr review-handoff recover --reason reviewer-declined` with the exact --expected-head and --expected-state; coordinator-retired takeover requires a distinct supervisor configured in [review_handoff].supervisors at the provider base; see forge-review-state-v1.md"
            } else {
                "recovery=the persisted reviewer can run `forge-cli pr review-handoff surrender` with the exact --expected-head and --expected-state; the original coordinator can instead run `forge-cli pr review-handoff recover --reason reviewer-declined` with those selectors, then assign the replacement; do not self-review"
            }
        }
        _ => "recovery=return control to the coordinator; do not self-review",
    }
}
