//! Typed admission is separate from the compatibility `reviewed` status.

use super::*;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ApprovalState {
    SourcePassed,
    FinallyApproved,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PublishedApproval {
    pub state: ApprovalState,
    review_id: String,
    review_url: String,
    review_author: String,
    native_state: String,
    reviewed_head: String,
}

impl ApprovalState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SourcePassed => "source-passed",
            Self::FinallyApproved => "finally-approved",
        }
    }
}

impl PublishedApproval {
    pub(super) fn from_review(review: &pr_reviews::NativeReviewSummary, head: &str) -> Self {
        Self {
            state: if review.state == "APPROVED" && review.commit_sha == head {
                ApprovalState::FinallyApproved
            } else {
                ApprovalState::SourcePassed
            },
            review_id: review.id.clone(),
            review_url: review.url.clone(),
            review_author: review.author.clone(),
            native_state: review.state.clone(),
            reviewed_head: review.commit_sha.clone(),
        }
    }
}

#[derive(Default)]
pub(super) struct Admission {
    pub approval: Option<PublishedApproval>,
    pub sync_carry_over: Option<SyncCarryOver>,
}

impl Admission {
    pub(super) fn require_policy(&self, policy: ApprovalPolicy) -> Result<(), ForgeError> {
        if policy == ApprovalPolicy::FinalApproval
            && self
                .approval
                .as_ref()
                .is_none_or(|a| a.state != ApprovalState::FinallyApproved)
        {
            return Err(fail(
                "designated_final_approval_required",
                "source review passed; a current-head designated APPROVED report is required for final delivery",
            ));
        }
        Ok(())
    }
}
