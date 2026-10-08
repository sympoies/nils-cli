//! Operator-only retirement for a runtime stopped outside its controller.
//! This deliberately does not weaken ordinary coordination liveness evidence.
use super::{idempotency_replay, lock_registry_observational, now_epoch, store_receipt, timestamp};
use crate::cli::BrokerRetireStoppedArgs;
use crate::{CliContext, CliError, SessionRecord, TmuxRuntimeIdentity};
use serde_json::{Value, json};
use std::fs;
use std::time::Duration;

const OPERATION: &str = "broker-retire-stopped";
const VERSION: &str = "agent-session.broker-retirement.v1";

pub(crate) fn retire(
    context: &CliContext,
    args: BrokerRetireStoppedArgs,
) -> Result<Value, CliError> {
    let id = args.session.clone();
    crate::lifecycle::attempt(context, &id, OPERATION, || retire_locked(context, args))
}

fn proof(step: &str, passed: bool) -> Value {
    json!({"step": step, "passed": passed})
}

fn retire_locked(context: &CliContext, args: BrokerRetireStoppedArgs) -> Result<Value, CliError> {
    super::validate_idempotency_key(&args.idempotency_key)?;
    let observed = crate::load_session_record(context, &args.session)?;
    let id = observed.id.clone();
    // Same ordering as resume: lifecycle lock, then coordination lock. Preview
    // uses the observational lock so it cannot renew claims or expire leases.
    let _session_lock = crate::acquire_session_record_lock(context, &id)?;
    let record = crate::load_session_record(context, &id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    let mut locked = lock_registry_observational(context)?;
    let runtime = record.runtime.as_ref();
    let selector_matches =
        runtime.is_some_and(|r| r.launch_id == args.incarnation && r.generation == args.generation);
    let broker = locked.registry.brokers.get(&id).cloned();
    let broker_matches = broker.as_ref().is_some_and(|b| {
        b.session_id == id && b.incarnation == args.incarnation && b.generation == args.generation
    });
    let digest = super::digest_bytes(&serde_json::to_vec(&json!({"session_id":id,"incarnation":args.incarnation,"generation":args.generation,"stale_after_seconds":args.stale_after})).map_err(|_| super::store_corrupt())?);
    // Local owner authority, not a runtime capability: a lost controller's
    // capability cannot attest that it has stopped.
    let principal = format!("operator-retirement:{id}");
    if selector_matches
        && broker_matches
        && let Some(receipt) = idempotency_replay(
            &locked.registry,
            &args.idempotency_key,
            &principal,
            &args.incarnation,
            OPERATION,
            &digest,
        )?
    {
        let broker = broker.as_ref().expect("matched broker");
        if broker.state != "stopped" || !broker.capability_digest.is_empty() {
            return Err(CliError::data(
                "session-incarnation-conflict",
                "retired broker was replaced before receipt replay",
                None,
            ));
        }
        if args.apply {
            cleanup(context, &record, &args.incarnation, &receipt)?;
        }
        return Ok(receipt);
    }
    let tmux = crate::resolve_tmux_bin(args.tmux_bin.as_deref());
    let identity = crate::persisted_tmux_runtime_identity(&record)
        .ok()
        .flatten();
    let evidence = crate::coordination_runtime_evidence(context, &record).ok();
    let exact_identity = identity
        .as_ref()
        .is_some_and(|identity| identity.launch_id.as_deref() == Some(args.incarnation.as_str()))
        && evidence
            .as_ref()
            .zip(broker.as_ref())
            .is_some_and(|(e, b)| {
                b.runtime_identity.as_ref() == Some(&e.identity)
                    && b.runtime_identity_digest == e.identity_digest
            });
    let same_boot_proven = identity.as_ref().is_some_and(same_boot);
    let tmux_absent = identity.as_ref().is_some_and(|identity| {
        crate::verified_tmux_status_with_timeout(
            &tmux,
            &identity.session_id,
            crate::DELETE_TERMINATION_VERIFY_TIMEOUT,
        ) == "stopped"
    });
    let pane_absent = identity
        .as_ref()
        .is_some_and(|identity| process_absent(identity.pane_pid));
    let group_absent = identity.as_ref().is_some_and(process_boundary_absent);
    let prior_absent = crate::persisted_prior_tmux_runtime_identities(&record).is_ok_and(|prior| {
        prior.iter().all(|identity| {
            crate::verify_stopped_tmux_runtime(&tmux, identity, Duration::ZERO).is_ok()
        })
    });
    let now = now_epoch();
    let heartbeat_stale = broker.as_ref().is_some_and(|broker| {
        heartbeat_is_stale(
            context,
            &id,
            &args.incarnation,
            broker.heartbeat_epoch,
            args.stale_after,
        )
    });
    let operations_quiescent = !locked.registry.operations.iter().any(|lease| {
        lease.session_id == id
            && lease.session_incarnation == args.incarnation
            && !matches!(lease.state.as_str(), "completed" | "abandoned")
    });
    let claims_unfenced = locked
        .registry
        .claims
        .iter()
        .filter(|claim| {
            claim.session_id == id
                && claim.session_incarnation == args.incarnation
                && claim.state == "active"
        })
        .all(|claim| {
            super::claim_mutation_fence_absent(context, &id, &args.incarnation, claim)
                .unwrap_or(false)
        });
    let lifecycle_unfenced = super::ensure_notification_submission_not_in_progress(
        &locked.registry,
        &id,
        &args.incarnation,
    )
    .is_ok()
        && crate::orchestration::ensure_session_not_authority_quarantined(context, &record).is_ok()
        && crate::orchestration::ensure_session_not_runtime_stop_fenced(context, &record).is_ok();
    let state_retirable = broker.as_ref().is_some_and(|b| {
        matches!(
            b.state.as_str(),
            "ready" | "degraded" | "starting" | "recovering"
        )
    });
    let proofs = vec![
        proof("session-selector", selector_matches),
        proof("broker-selector", broker_matches),
        proof("runtime-identity", exact_identity),
        proof("same-boot", same_boot_proven),
        proof("heartbeat-stale", heartbeat_stale),
        proof("tmux-target-absent", tmux_absent),
        proof("pane-process-absent", pane_absent),
        proof("process-group-absent", group_absent),
        proof("prior-runtime-boundaries-absent", prior_absent),
        proof("operations-quiescent", operations_quiescent),
        proof("claims-unfenced", claims_unfenced),
        proof("lifecycle-unfenced", lifecycle_unfenced),
        proof("broker-state", state_retirable),
    ];
    let eligible = proofs.iter().all(|p| p["passed"] == true);
    let preview = json!({"schema_version":VERSION,"session_id":id,"session_incarnation":args.incarnation,"session_generation":args.generation,"stale_after_seconds":args.stale_after,"eligible":eligible,"state":"preview","proofs":proofs});
    if !args.apply {
        return Ok(preview);
    }
    if !eligible {
        let code = if !selector_matches || !broker_matches {
            "session-incarnation-conflict"
        } else {
            "coordination-retirement-refused"
        };
        let step = preview["proofs"]
            .as_array()
            .and_then(|rows| rows.iter().find(|p| p["passed"] != true))
            .and_then(|p| p["step"].as_str())
            .unwrap_or("runtime-stopped-proof");
        return Err(CliError::data(
            code,
            "operator retirement requires every stopped-runtime proof to pass",
            Some(json!({"proof_step":step,"preview":preview})),
        ));
    }
    // These probes are not broker-lock protected (the kernel and tmux are
    // outside the registry). Sample them again immediately before the one save.
    let identity = identity.as_ref().expect("eligible identity");
    if !same_boot(identity)
        || !process_absent(identity.pane_pid)
        || !process_boundary_absent(identity)
        || crate::verified_tmux_status_with_timeout(
            &tmux,
            &identity.session_id,
            crate::DELETE_TERMINATION_VERIFY_TIMEOUT,
        ) != "stopped"
        || !broker.as_ref().is_some_and(|broker| {
            heartbeat_is_stale(
                context,
                &id,
                &args.incarnation,
                broker.heartbeat_epoch,
                args.stale_after,
            )
        })
    {
        return Err(CliError::data(
            "coordination-retirement-refused",
            "stopped-runtime proof changed before retirement",
            Some(json!({"proof_step":"runtime-stopped-proof"})),
        ));
    }
    let mut receipt = preview;
    receipt["state"] = json!("retired");
    receipt["retired_at"] = json!(timestamp(now));
    receipt["receipt_id"] = json!(super::digest_bytes(args.idempotency_key.as_bytes()));
    // Receipt capacity is checked before mutating the candidate registry.
    store_receipt(
        &mut locked.registry,
        args.idempotency_key,
        principal,
        args.incarnation.clone(),
        OPERATION.to_string(),
        digest,
        receipt.clone(),
        now,
    )?;
    let broker = locked
        .registry
        .brokers
        .get_mut(&id)
        .expect("eligible broker");
    broker.state = "stopped".to_string();
    broker.capability_digest.clear();
    broker.heartbeat_at = timestamp(now);
    broker.heartbeat_epoch = now;
    for claim in &mut locked.registry.claims {
        if claim.session_id == id
            && claim.session_incarnation == args.incarnation
            && claim.state == "active"
        {
            claim.state = "released".to_string();
            claim.revision = claim.revision.saturating_add(1);
            claim.updated_at = timestamp(now);
            claim.terminal_at_epoch = Some(now);
        }
    }
    super::broker::remove_advisory_state_for_incarnation(
        &mut locked.registry,
        &id,
        &args.incarnation,
    );
    locked.save()?;
    // The broker state, authentication digest revocation and replay receipt
    // commit atomically. Removing the private files is a replayable second step
    // under both locks; a failure never advertises a completed filesystem cleanup.
    cleanup(context, &record, &args.incarnation, &receipt)?;
    Ok(receipt)
}

fn process_absent(pid: libc::pid_t) -> bool {
    pid > 1
        && unsafe { libc::kill(pid, 0) } != 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}
fn same_boot(identity: &TmuxRuntimeIdentity) -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::linux_runtime_pid_namespace_relation(identity)
            == crate::LinuxPidNamespaceRelation::Same
    }
    #[cfg(target_os = "macos")]
    {
        identity
            .macos_boot_id
            .as_deref()
            .filter(|id| crate::valid_linux_boot_id(id))
            .zip(crate::capture_macos_boot_id().as_deref())
            .is_some_and(|(old, current)| old == current)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = identity;
        false
    }
}
fn process_boundary_absent(identity: &TmuxRuntimeIdentity) -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::coordination_process_runtime_status(identity) == crate::ProcessGroupStatus::Stopped
    }
    #[cfg(target_os = "macos")]
    {
        super::broker::macos_process_group_is_empty(identity)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = identity;
        false
    }
}
fn cleanup(
    context: &CliContext,
    record: &SessionRecord,
    incarnation: &str,
    receipt: &Value,
) -> Result<(), CliError> {
    for path in [
        super::capability_path(context, &record.id, incarnation),
        super::heartbeat_path(&context.state_dir, &record.id),
        super::checkpoint_path_for_state(&context.state_dir, &record.id, incarnation),
    ] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(CliError::runtime(
                    "coordination-retirement-cleanup-pending",
                    "broker capability is revoked; retry the same retirement request to finish private-file cleanup",
                    Some(json!({"receipt":receipt,"proof_step":"capability-present"})),
                ));
            }
        }
    }
    Ok(())
}

fn heartbeat_is_stale(
    context: &CliContext,
    id: &str,
    incarnation: &str,
    broker_epoch: i64,
    threshold: u64,
) -> bool {
    let now = now_epoch();
    if broker_epoch <= 0
        || broker_epoch > now
        || now.saturating_sub(broker_epoch) < threshold as i64
    {
        return false;
    }
    let heartbeat = super::heartbeat_path(&context.state_dir, id);
    match fs::symlink_metadata(heartbeat) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
        Ok(_) => nils_common::coordination_projection::heartbeat_age_seconds(
            &context.state_dir,
            id,
            incarnation,
            now,
        )
        .is_some_and(|age| age >= threshold as i64 && age < now),
    }
}
