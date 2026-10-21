//! Compare-and-swap append logic for the review-state ledger.

use super::{
    ReviewLoopAppendResult, build_issue_comment_call, read_review_state_snapshot, schema_err,
};
use crate::backend::BackendRunner;
use crate::error::ForgeError;
use crate::ops::review_state;
use crate::provider::ProviderContext;

pub(crate) struct ReviewStateAppend<'a> {
    pub repository: &'a str,
    pub number: u64,
    pub expected_head: &'a str,
    pub expected_tip: Option<&'a str>,
    pub payload: review_state::ReviewStatePayload,
    pub visible_outcome: Option<&'a str>,
    pub policy_remote: Option<&'a str>,
}

pub(crate) fn append_review_state_payload<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    append: ReviewStateAppend<'_>,
) -> Result<ReviewLoopAppendResult, ForgeError> {
    let ReviewStateAppend {
        repository,
        number,
        expected_head,
        expected_tip,
        payload,
        visible_outcome,
        policy_remote,
    } = append;
    let before = read_review_state_snapshot(runner, ctx, repository, number)?;
    if before.chain.tip_digest.as_deref() != expected_tip {
        return Err(ForgeError::validation(
            schema_err(),
            "review_state_conflict",
            "the review-state tip changed before append",
            Some(format!(
                "expected_tip={}; provider_tip={}",
                expected_tip.unwrap_or("<genesis>"),
                before.chain.tip_digest.as_deref().unwrap_or("<genesis>")
            )),
        ));
    }
    if let review_state::ReviewStatePayload::ReviewHandoff { handoff } = &payload {
        super::super::pr_review_handoff::ensure_handoff_append(
            &before.chain,
            handoff,
            ctx,
            policy_remote.unwrap_or("origin"),
        )?;
        // Assignment and recovery bind a new interval to the provider scope;
        // surrender deliberately retains the old scope for recovery.
        if !handoff.surrendered {
            let current = super::super::pr_view::compute(runner, ctx, number)?;
            if current.head_sha.as_deref() != Some(expected_head) {
                return Err(ForgeError::validation(
                    schema_err(),
                    "review_state_conflict",
                    "provider head changed before review handoff",
                    None,
                ));
            }
            if super::super::pr_review_handoff::provider_base(runner, ctx, number)?
                != handoff.base_sha
            {
                return Err(ForgeError::validation(
                    schema_err(),
                    "review_scope_changed",
                    "provider base changed before review handoff",
                    None,
                ));
            }
        }
    } else {
        super::super::pr_review_handoff::ensure_payload_writer(&before.chain, &payload)?;
    }
    let assignment_generation = match &payload {
        review_state::ReviewStatePayload::ReviewHandoff { handoff } => {
            Some(handoff.assignment_generation)
        }
        _ => {
            super::super::pr_review_handoff::latest(&before.chain).map(|h| h.assignment_generation)
        }
    };
    let record = review_state::ReviewStateRecord::new(
        repository,
        number,
        expected_head,
        before.chain.records.len() as u64,
        before.chain.tip_digest.clone(),
        payload,
    )?
    .with_assignment_generation(assignment_generation)?;
    let marker = record.marker()?;
    let body = review_state::render_state_comment_body(&record, visible_outcome)?;
    let append_result = runner.run(&build_issue_comment_call(ctx, number, &body));
    let after = read_review_state_snapshot(runner, ctx, repository, number)?;
    if after
        .chain
        .records
        .iter()
        .any(|observed| observed.record_digest == record.record_digest)
    {
        // The record is durable. When an outcome shares the comment, confirm the
        // outcome itself is visible too: another session may have made the
        // record durable without publishing this outcome.
        let outcome_posted = match visible_outcome {
            None => false,
            Some(outcome) => {
                let posted = after.trusted_comments.iter().any(|comment| {
                    review_state::comment_carries_outcome(&comment.body, &marker, outcome)
                });
                if !posted {
                    return Err(ForgeError::validation(
                        schema_err(),
                        "review_outcome_not_posted",
                        "the review-loop record is durable but its delivery outcome is not visible after provider write",
                        Some(format!(
                            "record_digest={}; recovery=inspect the pull request and post the outcome separately, or rerun the identical observe against the new tip",
                            record.record_digest
                        )),
                    ));
                }
                true
            }
        };
        return Ok(ReviewLoopAppendResult {
            chain: after.chain,
            outcome_posted,
        });
    }
    Err(match append_result {
        Ok(_) => ForgeError::validation(
            schema_err(),
            "review_state_conflict",
            "the review-loop state was not visible after provider write",
            Some(format!("record_digest={}", record.record_digest)),
        ),
        Err(error) => error,
    })
}
