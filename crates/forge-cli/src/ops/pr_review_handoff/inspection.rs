//! Read-only publication projection using the check gate's semantic owner.

use super::*;

pub(super) fn published<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
    url: &str,
    head: &str,
    chain: &review_state::ReviewStateChain,
    handoff_created_at: Option<&str>,
) -> Result<Option<PublishedApproval>, ForgeError> {
    let admission = admit_published(runner, ctx, number, url, head, chain, handoff_created_at)
        .and_then(|admission| {
            admission
                .require_policy(latest(chain).map(|h| h.approval_policy).unwrap_or_default())?;
            Ok(admission)
        });
    match admission {
        Ok(admission) => Ok(admission.approval),
        // Inspection still exposes ownership while an ordinary admission is
        // pending. Provider and incomplete-snapshot errors remain failures.
        Err(error)
            if matches!(
                error.kind(),
                "awaiting_designated_review" | "designated_final_approval_required"
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
