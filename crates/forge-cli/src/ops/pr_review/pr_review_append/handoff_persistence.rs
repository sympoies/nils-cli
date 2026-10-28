//! Preserve the identity of an acknowledged handoff write if read-back fails.
use crate::error::ForgeError;
use crate::ops::review_state::{ReviewStatePayload, ReviewStateRecord};

pub(super) fn verification_error(
    record: &ReviewStateRecord,
    acknowledged: bool,
    cause: ForgeError,
) -> ForgeError {
    if !acknowledged || !matches!(record.payload, ReviewStatePayload::ReviewHandoff { .. }) {
        return cause;
    }
    ForgeError::validation_with_details(
        "cli.forge-cli.pr.review-handoff.v1",
        "review_handoff_persisted_verification_failed",
        format!(
            "the review handoff was persisted at resulting tip {}; post-write verification failed ({}); inspect the handoff before retrying",
            record.record_digest,
            cause.kind(),
        ),
        serde_json::json!({
            "persisted": true,
            "state_tip_digest": record.record_digest,
            "handoff_digest": record.record_digest,
            "retryable": false,
            "next_action": "inspect_review_handoff",
            "cause": { "code": cause.kind() },
        }),
    )
}
