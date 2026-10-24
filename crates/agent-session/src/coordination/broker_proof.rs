//! Exact evidence only: never admit, replay, complete, or run registry maintenance.
use super::{Registry, claims::OperationLease};
use crate::{
    CliContext, CliError,
    cli::{
        WorkContextAdmitArgs,
        broker_proof::{BrokerProofArgs, PrepareAdmissionProofArgs},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_SELECTORS: usize = 128;
const MAX_BATCH_BYTES: u64 = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    schema_version: String,
    selectors: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Selector {
    Operation {
        lease_id: String,
        revision_floor: u64,
        claim_id: String,
        claim_revision: u64,
    },
    Admission {
        admit_idempotency_key: String,
        claim_id: String,
        claim_revision: u64,
        expected_request_digest: String,
    },
}

fn invalid() -> CliError {
    CliError::data(
        "invalid-broker-proof",
        "broker proof input is invalid",
        None,
    )
}
fn conflict(code: &'static str) -> CliError {
    CliError::data(
        code,
        "exact broker proof selector conflicts with retained evidence",
        None,
    )
}
fn bounded_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

pub(super) fn prepare(
    context: &CliContext,
    args: PrepareAdmissionProofArgs,
) -> Result<Value, CliError> {
    let session = args
        .session
        .or_else(|| std::env::var("AGENT_SESSION_ID").ok())
        .ok_or_else(super::unauthorized)?;
    let token = super::capability_token_from_file(args.capability_file.as_deref())?;
    let authority =
        crate::lock_exact_session_authority(context, &session)?.ok_or_else(super::unauthorized)?;
    let record = &authority.record;
    let incarnation = super::incarnation(record)?;
    let locked = super::lock_registry_observational(context)?;
    super::verify_capability_token(context, &locked.registry, record, &incarnation, &token)?;
    super::validate_idempotency_key(&args.admit_idempotency_key)?;
    if !bounded_id(&args.claim) || args.claim_revision == 0 {
        return Err(invalid());
    }
    let request = super::claims::admission_request::prepare(&WorkContextAdmitArgs {
        session,
        capability_file: args.capability_file,
        claim: args.claim.clone(),
        if_revision: args.claim_revision,
        operation: args.operation,
        targets_file: args.targets_file,
        execution_token_file: args.execution_token_file,
        idempotency_key: args.admit_idempotency_key.clone(),
        format: args.format,
    })?;
    Ok(json!({
        "schema_version":"agent-session.broker-admission-proof-preparation.v1",
        "selector": { "kind":"admission", "admit_idempotency_key":args.admit_idempotency_key,
            "claim_id":args.claim, "claim_revision":args.claim_revision, "expected_request_digest":request.digest }
    }))
}

pub(super) fn proof(context: &CliContext, args: BrokerProofArgs) -> Result<Value, CliError> {
    let token = super::capability_token_from_file(args.capability_file.as_deref())?;
    let authority = crate::lock_exact_session_authority(context, &args.session)?
        .ok_or_else(super::unauthorized)?;
    let record = &authority.record;
    let incarnation = super::incarnation(record)?;
    let locked = super::lock_registry_observational(context)?;
    super::verify_capability_token(context, &locked.registry, record, &incarnation, &token)?;
    let broker = locked
        .registry
        .brokers
        .get(&record.id)
        .ok_or_else(super::unauthorized)?;
    if incarnation != args.expected_incarnation {
        return Err(conflict("session-incarnation-conflict"));
    }
    if args.expected_generation == 0
        || broker.generation != args.expected_generation
        || record
            .runtime
            .as_ref()
            .is_none_or(|r| r.generation != args.expected_generation)
    {
        return Err(conflict("broker-generation-conflict"));
    }
    // Input and output are both bounded. Parse items separately so one malformed
    // selector cannot suppress another lease's exact evidence.
    let bytes =
        super::read_private_file(&args.batch_file, MAX_BATCH_BYTES).map_err(|_| invalid())?;
    let batch: Batch = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if batch.schema_version != "agent-session.broker-proof-batch.v1"
        || batch.selectors.is_empty()
        || batch.selectors.len() > MAX_SELECTORS
    {
        return Err(invalid());
    }
    let now = super::now_epoch();
    let proofs: Vec<Value> = batch.selectors.into_iter().enumerate().map(|(index, value)| {
        let result = serde_json::from_value(value).map_err(|_| invalid())
            .and_then(|selector| inspect(&locked.registry, &record.id, &incarnation, now, selector));
        match result {
            Ok(proof) => json!({"index":index, "ok":true, "proof":proof}),
            Err(error) => json!({"index":index, "ok":false, "error":{"code":error.code(), "message":error.message()}}),
        }
    }).collect();
    Ok(
        json!({"schema_version":"agent-session.broker-proof.v1", "session_id":record.id,
        "session_incarnation":incarnation, "generation":broker.generation, "proofs":proofs}),
    )
}

fn terminal(state: &str) -> bool {
    matches!(state, "completed" | "failed" | "abandoned" | "expired")
}
fn known(state: &str) -> bool {
    terminal(state) || matches!(state, "active" | "completing" | "reconcile_pending")
}

fn inspect(
    registry: &Registry,
    session: &str,
    incarnation: &str,
    now: i64,
    selector: Selector,
) -> Result<Value, CliError> {
    match selector {
        Selector::Operation {
            lease_id,
            revision_floor,
            claim_id,
            claim_revision,
        } => {
            if !bounded_id(&lease_id)
                || !bounded_id(&claim_id)
                || revision_floor == 0
                || claim_revision == 0
            {
                return Err(invalid());
            }
            let mut proof = json!({"kind":"operation", "lease_id":lease_id, "found":false,
                "provenance":"not_retained", "claim":{"claim_id":claim_id,"revision":claim_revision}});
            if let Some(lease) = retained(registry, session, incarnation, &lease_id) {
                check_binding(lease, &claim_id, claim_revision, revision_floor)?;
                if known(&lease.state) {
                    proof = operation_proof(registry, lease, now);
                }
            } else if let Some(receipt) =
                terminal_receipt(registry, session, incarnation, &lease_id, now)
            {
                check_receipt_binding(receipt, &claim_id, claim_revision, revision_floor)?;
                proof = json!({"kind":"operation", "found":true, "lease_id":lease_id,
                    "claim":{"claim_id":claim_id,"revision":claim_revision}, "revision":receipt["revision"],
                    "state":receipt["state"], "outcome":receipt["outcome"], "provenance":"historical_receipt"});
            }
            Ok(proof)
        }
        Selector::Admission {
            admit_idempotency_key,
            claim_id,
            claim_revision,
            expected_request_digest,
        } => {
            super::validate_idempotency_key(&admit_idempotency_key)?;
            if !bounded_id(&claim_id)
                || claim_revision == 0
                || expected_request_digest.len() != 64
                || !expected_request_digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid());
            }
            let unknown = json!({"kind":"admission", "status":"unknown", "provenance":"not_retained",
                "retained":false, "claim":{"claim_id":claim_id,"revision":claim_revision}});
            let key = super::receipt_key(
                session,
                incarnation,
                "work-context-admit",
                &admit_idempotency_key,
            );
            let Some(receipt) = registry
                .receipts
                .get(&key)
                .filter(|r| r.expires_at_epoch > now)
            else {
                return Ok(unknown);
            };
            if receipt.principal != session
                || receipt.incarnation != incarnation
                || receipt.operation != "work-context-admit"
                || receipt.digest != expected_request_digest
            {
                return Err(conflict("admission-request-conflict"));
            }
            let issued = &receipt.outcome;
            check_receipt_binding(issued, &claim_id, claim_revision, 1)?;
            let Some(lease_id) = issued["lease_id"].as_str().filter(|id| bounded_id(id)) else {
                return Ok(unknown);
            };
            if issued["session_id"] != session || issued["session_incarnation"] != incarnation {
                return Ok(unknown);
            }
            if let Some(lease) = retained(registry, session, incarnation, lease_id) {
                check_binding(
                    lease,
                    &claim_id,
                    claim_revision,
                    issued["revision"].as_u64().unwrap_or(1),
                )?;
                if !known(&lease.state) {
                    return Ok(unknown);
                }
                let mut proof = operation_proof(registry, lease, now);
                proof["kind"] = json!("admission");
                proof["status"] = json!("committed");
                proof["retained"] = json!(true);
                return Ok(proof);
            }
            // An admission receipt alone proves issuance, never terminality.
            // Require an independently persisted exact terminal completion/reconcile receipt.
            if let Some(terminal) = terminal_receipt(registry, session, incarnation, lease_id, now)
            {
                check_receipt_binding(
                    terminal,
                    &claim_id,
                    claim_revision,
                    issued["revision"].as_u64().unwrap_or(1),
                )?;
                return Ok(
                    json!({"kind":"admission", "status":"committed", "provenance":"historical_receipt",
                    "retained":false,"lease_id":lease_id,"claim":{"claim_id":claim_id,"revision":claim_revision},
                    "state":null,"revision":null,"outcome":null}),
                );
            }
            Ok(unknown)
        }
    }
}

fn retained<'a>(
    registry: &'a Registry,
    session: &str,
    incarnation: &str,
    lease_id: &str,
) -> Option<&'a OperationLease> {
    registry.operations.iter().find(|l| {
        l.session_id == session && l.session_incarnation == incarnation && l.lease_id == lease_id
    })
}
fn check_binding(
    lease: &OperationLease,
    claim: &str,
    revision: u64,
    floor: u64,
) -> Result<(), CliError> {
    if lease.claim_id != claim || lease.claim_revision != revision {
        return Err(conflict("claim-revision-conflict"));
    }
    if lease.revision < floor {
        return Err(conflict("operation-revision-conflict"));
    }
    Ok(())
}
fn check_receipt_binding(
    receipt: &Value,
    claim: &str,
    revision: u64,
    floor: u64,
) -> Result<(), CliError> {
    if receipt["claim_id"] != claim || receipt["claim_revision"] != revision {
        return Err(conflict("claim-revision-conflict"));
    }
    if receipt["revision"].as_u64().is_none_or(|r| r < floor) {
        return Err(conflict("operation-revision-conflict"));
    }
    Ok(())
}
fn operation_proof(registry: &Registry, lease: &OperationLease, now: i64) -> Value {
    // A safety deadline or already queued completion cannot restore execution
    // authority even if observational reads precede the next maintenance tick.
    let state = if lease.state == "active"
        && (lease.expires_at_epoch <= now
            || registry.completion_events.iter().any(|e| {
                e.session_id == lease.session_id
                    && e.session_incarnation == lease.session_incarnation
                    && e.lease_id == lease.lease_id
            })) {
        "completing"
    } else {
        &lease.state
    };
    json!({"kind":"operation","found":true,"lease_id":lease.lease_id,"revision":lease.revision,
        "claim":{"claim_id":lease.claim_id,"revision":lease.claim_revision},"state":state,
        "outcome":lease.outcome,"provenance":"retained_operation"})
}
fn terminal_receipt<'a>(
    registry: &'a Registry,
    session: &str,
    incarnation: &str,
    lease_id: &str,
    now: i64,
) -> Option<&'a Value> {
    registry
        .receipts
        .values()
        .filter(|r| {
            r.principal == session
                && r.incarnation == incarnation
                && r.expires_at_epoch > now
                && matches!(
                    r.operation.as_str(),
                    "work-context-complete" | "work-context-reconcile"
                )
                && r.outcome["session_id"] == session
                && r.outcome["session_incarnation"] == incarnation
                && r.outcome["lease_id"] == lease_id
                && r.outcome["state"].as_str().is_some_and(terminal)
        })
        .max_by_key(|r| r.outcome["revision"].as_u64())
        .map(|r| &r.outcome)
}
