//! Shared Main Agent orchestration guards and receipt helpers.
//!
//! Both the `main-agent` facade and the group lifecycle engine use these helpers:
//! assignment mutation admission, in-flight fences, idempotency receipts,
//! revision fences, and common error constructors.

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use jiff::Zoned;
use serde_json::{Value, json};

use crate::orchestration::{self, AssignmentRecord, IdempotencyReceipt, SessionRef};
use crate::{CliContext, CliError, SessionRecord};

pub const MAX_IDEMPOTENCY_RECEIPTS: usize = 32_768;
#[cfg(test)]
pub static IDEMPOTENCY_RECEIPT_CAPACITY_FOR_TEST: AtomicUsize =
    AtomicUsize::new(MAX_IDEMPOTENCY_RECEIPTS);
pub fn ensure_submit_recovery_not_in_flight(assignment: &AssignmentRecord) -> Result<(), CliError> {
    if submit_recovery_in_flight(assignment) {
        return Err(CliError::data(
            "submit-recovery-in-flight",
            "assignment mutation is fenced until the reserved recovery attempt is resolved",
            Some(json!({
                "assignment_id": assignment.assignment_id,
                "revision": assignment.revision
            })),
        ));
    }
    Ok(())
}

pub fn submit_recovery_in_flight(assignment: &AssignmentRecord) -> bool {
    assignment.submit_recovery.as_ref().is_some_and(|recovery| {
        matches!(recovery.state.as_str(), "attempting" | "sent")
            && assignment
                .checkpoint
                .as_ref()
                .is_none_or(|checkpoint| checkpoint.revision <= recovery.reserved_revision)
    })
}

pub fn account_handoff_in_flight(assignment: &AssignmentRecord) -> CliError {
    CliError::data(
        "account-handoff-in-flight",
        "assignment mutation is fenced until the reserved account handoff is resolved",
        Some(json!({
            "assignment_id": assignment.assignment_id,
            "revision": assignment.revision
        })),
    )
}

pub fn ensure_account_handoff_not_in_flight(assignment: &AssignmentRecord) -> Result<(), CliError> {
    if assignment.account_handoff.is_some() {
        return Err(account_handoff_in_flight(assignment));
    }
    Ok(())
}

pub fn worker_runtime_stop_in_flight(assignment: &AssignmentRecord) -> CliError {
    CliError::unavailable(
        "worker-runtime-stop-in-flight",
        "assignment mutation is fenced until the exact worker runtime stop completes or its durable receipt is replayed",
        Some(json!({
            "assignment_id": assignment.assignment_id,
            "revision": assignment.revision
        })),
    )
}

pub fn ensure_worker_runtime_stop_not_in_flight(
    assignment: &AssignmentRecord,
) -> Result<(), CliError> {
    if assignment.runtime_stop.is_some() {
        return Err(worker_runtime_stop_in_flight(assignment));
    }
    Ok(())
}

pub fn worker_claim_revocation_in_flight(assignment: &AssignmentRecord) -> CliError {
    CliError::unavailable(
        "worker-claim-revocation-in-flight",
        "assignment mutation is fenced until the exact worker claim revocation completes or its durable receipt is replayed",
        Some(json!({
            "assignment_id": assignment.assignment_id,
            "revision": assignment.revision
        })),
    )
}

pub fn ensure_worker_claim_revocation_not_in_flight(
    assignment: &AssignmentRecord,
) -> Result<(), CliError> {
    if assignment.claim_revocation.is_some() {
        return Err(worker_claim_revocation_in_flight(assignment));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignmentMutationOwner {
    Ordinary,
    StoppedReconciliation,
    AccountHandoff,
    RuntimeStop,
    ClaimedRuntimeStop,
    ClaimRevocation,
    ProviderStopCanary,
}

pub fn ensure_assignment_mutation_admitted(
    context: &CliContext,
    assignment: &AssignmentRecord,
    owner: AssignmentMutationOwner,
) -> Result<(), CliError> {
    if owner != AssignmentMutationOwner::ProviderStopCanary
        && orchestration::provider_stop_canary_reservation(context, assignment)?.is_some()
    {
        // Stopped reconciliation re-establishes exact process and tmux proof
        // under the session-record lock before committing. Let that sole
        // cleanup owner reach its stronger checks; every other mutation stays
        // fenced without relying on an unlocked preliminary runtime sample.
        if owner != AssignmentMutationOwner::StoppedReconciliation {
            return Err(CliError::unavailable(
                "provider-stop-canary-in-flight",
                "assignment mutation is fenced until the exact provider stop canary is released",
                Some(json!({
                    "assignment_id": assignment.assignment_id,
                    "revision": assignment.revision
                })),
            ));
        }
    }
    if orchestration::assignment_worker_reentry_in_progress(context, assignment)? {
        return Err(CliError::unavailable(
            "worker-reentry-in-flight",
            "assignment mutation is fenced until the original exact worker re-entry completes",
            Some(json!({
                "retryable": true,
                "next_action": "replay-original-worker-reenter",
                "recovery": {
                    "kind": "worker-reenter-replay",
                    "owner": "main-agent",
                    "automatic": false
                }
            })),
        ));
    }
    if orchestration::assignment_worker_delete_in_progress(context, assignment)? {
        return Err(CliError::unavailable(
            "worker-delete-in-flight",
            "assignment mutation is fenced until the original exact worker delete completes",
            Some(json!({
                "assignment_id": assignment.assignment_id,
                "revision": assignment.revision
            })),
        ));
    }
    if owner != AssignmentMutationOwner::AccountHandoff {
        ensure_account_handoff_not_in_flight(assignment)?;
    }
    if owner != AssignmentMutationOwner::RuntimeStop {
        ensure_worker_runtime_stop_not_in_flight(assignment)?;
    }
    if !matches!(
        owner,
        AssignmentMutationOwner::RuntimeStop | AssignmentMutationOwner::ClaimedRuntimeStop
    ) && orchestration::assignment_runtime_stop_fence_in_progress(context, assignment)?
    {
        return Err(worker_runtime_stop_in_flight(assignment));
    }
    if owner != AssignmentMutationOwner::ClaimRevocation {
        ensure_worker_claim_revocation_not_in_flight(assignment)?;
    }
    Ok(())
}

pub fn session_ref(context: &CliContext, record: &SessionRecord, incarnation: &str) -> SessionRef {
    SessionRef {
        machine: context.host.clone(),
        session_id: record.id.clone(),
        session_incarnation: incarnation.to_string(),
        session_created_at: record.created_at.clone(),
    }
}

pub fn receipt_key(session_id: &str, incarnation: &str, idempotency_key: &str) -> String {
    format!("{session_id}:{incarnation}:{idempotency_key}")
}

#[allow(clippy::too_many_arguments)]
pub fn store_receipt_for_principal(
    registry: &mut orchestration::Registry,
    principal_session_id: &str,
    incarnation: &str,
    idempotency_key: &str,
    operation: &str,
    request_digest: &str,
    outcome: Value,
) -> Result<(), CliError> {
    let key = receipt_key(principal_session_id, incarnation, idempotency_key);
    if !registry.receipts.contains_key(&key)
        && registry.receipts.len() >= idempotency_receipt_capacity()
    {
        let oldest = registry
            .receipts
            .iter()
            .min_by_key(|(_, receipt)| receipt.created_at_epoch)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            registry.receipts.remove(&oldest);
        }
    }
    registry.receipts.insert(
        key,
        IdempotencyReceipt {
            principal_session_id: principal_session_id.to_string(),
            principal_incarnation: incarnation.to_string(),
            operation: operation.to_string(),
            request_digest: request_digest.to_string(),
            outcome,
            created_at_epoch: crate::coordination::now_epoch(),
        },
    );
    Ok(())
}

pub fn idempotency_receipt_capacity() -> usize {
    #[cfg(test)]
    {
        IDEMPOTENCY_RECEIPT_CAPACITY_FOR_TEST.load(Ordering::Acquire)
    }
    #[cfg(not(test))]
    {
        MAX_IDEMPOTENCY_RECEIPTS
    }
}

pub fn validate_idempotency_key(value: &str) -> Result<(), CliError> {
    orchestration::validate_slug("idempotency key", value, 128)
}

pub fn ensure_revision(expected: u64, actual: u64, resource: &str) -> Result<(), CliError> {
    if expected == actual {
        Ok(())
    } else {
        Err(CliError::data(
            "orchestration-revision-conflict",
            "orchestration revision fence did not match",
            Some(json!({ "resource": resource, "current_revision": actual })),
        ))
    }
}

pub fn timestamp() -> String {
    Zoned::now().timestamp().to_string()
}

pub fn invalid_input(message: &str) -> CliError {
    CliError::data("invalid-orchestration-input", message, None)
}

pub fn not_found(code: &'static str, message: &'static str) -> CliError {
    CliError::data(code, message, None)
}
