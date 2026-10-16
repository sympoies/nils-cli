use super::*;
use serde::{Deserialize, Serialize};

/// Explicit designated reviewer ownership and current-head publication gate.
#[derive(Args, Debug, Clone)]
pub struct PrReviewHandoffArgs {
    #[command(subcommand)]
    pub command: PrReviewHandoffCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum PrReviewHandoffCommand {
    /// Persist or explicitly reassign review ownership with head/tip CAS.
    Assign(PrReviewHandoffAssignArgs),
    /// Read the persisted handoff without revealing mailbox addresses.
    Inspect(PrReviewHandoffInspectArgs),
    /// Require a published current-head passing review and closed ledger.
    Check(PrReviewHandoffCheckArgs),
    /// Reviewer-owned surrender of the current assignment generation.
    Surrender(PrReviewHandoffSurrenderArgs),
    /// Explicit coordinator revocation with a required reason and fresh generation.
    Recover(PrReviewHandoffReturnArgs),
}

#[derive(Args, Debug, Clone)]
pub struct PrReviewHandoffInspectArgs {
    pub id: u64,
    /// Exact provider head SHA for this handoff operation.
    #[arg(long)]
    pub expected_head: Option<String>,
    /// Expected public native review author of an existing assignment.
    #[arg(long)]
    pub review_author: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PrReviewHandoffSurrenderArgs {
    pub id: u64,
    /// Exact provider head SHA for this handoff operation.
    #[arg(long)]
    pub expected_head: String,
    /// Exact retained provider ledger tip for the ownership transition.
    #[arg(long)]
    pub expected_state: String,
}

#[derive(Args, Debug, Clone)]
pub struct PrReviewHandoffCheckArgs {
    /// Require this approval level in addition to the persisted assignment policy.
    #[arg(long, value_enum)]
    pub approval_policy: Option<ApprovalPolicy>,
    pub id: u64,
    /// Exact provider head SHA for this handoff operation.
    #[arg(long)]
    pub expected_head: String,
}

#[derive(Args, Debug, Clone)]
pub struct PrReviewHandoffAssignArgs {
    /// Approval required by check, assigned delivery and merge; defaults to source-pass.
    /// Omission preserves the existing assignment policy during handover or refresh.
    #[arg(long, value_enum)]
    pub approval_policy: Option<ApprovalPolicy>,
    pub id: u64,
    /// Private session selector; only its session digest is published.
    #[arg(long)]
    pub reviewer_session: String,
    /// Expected public author of the native governed or portable review.
    #[arg(long)]
    pub review_author: String,
    /// Exact provider base SHA captured for the assignment.
    #[arg(long)]
    pub base_sha: String,
    /// Exact provider head SHA for this handoff operation.
    #[arg(long)]
    pub expected_head: String,
    /// Exact ledger tip, or none for genesis. Required even for reassignment.
    #[arg(long)]
    pub expected_state: String,
}

#[derive(Args, Debug, Clone)]
pub struct PrReviewHandoffReturnArgs {
    pub id: u64,
    /// Exact provider head SHA for this handoff operation.
    #[arg(long)]
    pub expected_head: String,
    /// Exact retained provider ledger tip for the ownership transition.
    #[arg(long)]
    pub expected_state: String,
    /// Bounded role-based reason; mailbox details stay private.
    #[arg(long, value_parser = ["reviewer-unreachable", "reviewer-closed", "reviewer-declined", "reviewer-timeout", "coordinator-retired"])]
    pub reason: String,
    /// Recorded coordinator selector; required for coordinator-retired only.
    #[arg(long, required_if_eq("reason", "coordinator-retired"))]
    pub coordinator_session: Option<String>,
    /// Exact current provider base; required for coordinator-retired only.
    #[arg(long, required_if_eq("reason", "coordinator-retired"))]
    pub base_sha: Option<String>,
}

/// Source closure is the compatibility default; final approval is opt-in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    #[default]
    SourcePass,
    FinalApproval,
}

impl ApprovalPolicy {
    pub fn is_source_pass(&self) -> bool {
        *self == Self::SourcePass
    }

    pub fn requiring(self, requested: Option<Self>) -> Self {
        if self == Self::FinalApproval || requested == Some(Self::FinalApproval) {
            Self::FinalApproval
        } else {
            Self::SourcePass
        }
    }
}
