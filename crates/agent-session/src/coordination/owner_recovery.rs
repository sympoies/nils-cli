//! Runtime-local recovery for `work-context set --if-absent` only.
//! Launch selectors identify a target; kernel-observed ancestry authenticates it.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::coordination::{
    CAPABILITY_ENV, broker, capability, capability_path, digest_bytes, digest_eq, incarnation,
    lock_registry_observational, now_epoch, read_private_file, timestamp,
};
use crate::{CliContext, CliError, CoordinationRuntimeStatus, SessionRecord};

pub(super) fn authenticate(context: &CliContext) -> Result<(SessionRecord, String), CliError> {
    match super::require_self(context) {
        Ok(owner) => Ok(owner),
        Err(error)
            if error.code() == "coordination-broker-lost"
                || error.details().is_some_and(|details| {
                    matches!(
                        details["reason_code"].as_str(),
                        Some("capability-file-invalid" | "broker-not-ready")
                    )
                }) =>
        {
            let id = crate::non_empty_env("AGENT_SESSION_ID")
                .ok_or_else(|| refused("owner-recovery-identity-missing"))?;
            crate::validate_id(&id).map_err(|_| refused("owner-recovery-identity-invalid"))?;
            let mut kind = "owner-capability-reissue";
            let result = recover(context, &id, &mut kind);
            let record = crate::load_session_record(context, &id).ok();
            crate::lifecycle::record(
                context,
                record.as_ref(),
                &id,
                kind,
                "cli",
                result.as_ref().map(|_| ()),
                None,
            );
            result
        }
        Err(error) => Err(error),
    }
}

fn refused(reason: &'static str) -> CliError {
    capability::refusal(reason)
}

fn heartbeat_unavailable() -> CliError {
    let mut error = refused("owner-recovery-heartbeat-unavailable").with_hint(
        "owner capability restoration is incomplete; inside the exact current runtime, retry `work-context set --if-absent` once the heartbeat owner is available; preserve the session and restored capability",
    );
    if let Some(details) = error.details_mut() {
        details["retryable"] = serde_json::json!(true);
        details["next_action"] = serde_json::json!("retry-exact-runtime-owner");
        details["recovery"]["action"] = serde_json::json!("retry-exact-runtime-owner");
    }
    error
}

fn recover(
    context: &CliContext,
    id: &str,
    kind: &mut &'static str,
) -> Result<(SessionRecord, String), CliError> {
    // This route cannot wait indefinitely behind an in-flight controller transition.
    let _session = crate::acquire_session_record_lock_timed(context, id, Duration::from_secs(2))
        .map_err(|_| refused("owner-recovery-session-busy"))?;
    let record = crate::load_session_record(context, id)
        .map_err(|_| refused("owner-recovery-session-unavailable"))?;
    let current = incarnation(&record)?;
    let generation = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.generation)
        .unwrap_or(0);
    if crate::non_empty_env("AGENT_SESSION_RUNTIME_ID").as_deref() != Some(current.as_str()) {
        return Err(refused("owner-recovery-stale-runtime"));
    }
    let path = capability_path(context, id, &current);
    if std::env::var_os(CAPABILITY_ENV).map(PathBuf::from).as_ref() != Some(&path) {
        return Err(refused("owner-recovery-selector-mismatch"));
    }
    prove_caller(&record)?;
    let runtime = crate::coordination_runtime_evidence(context, &record)
        .map_err(|_| refused("owner-recovery-runtime-unverified"))?;
    if runtime.status != CoordinationRuntimeStatus::Running {
        return Err(refused("owner-recovery-runtime-unverified"));
    }
    // Observational acquisition avoids maintenance changing state on a refusal.
    let mut locked = lock_registry_observational(context)?;
    let previous = locked
        .registry
        .brokers
        .get(id)
        .cloned()
        .ok_or_else(|| refused("owner-recovery-broker-unavailable"))?;
    if previous.session_id != id
        || !matches!(previous.state.as_str(), "ready" | "recovering")
        || previous.capability_digest.is_empty()
    {
        return Err(refused("owner-recovery-broker-revoked"));
    }
    let rebind = previous.incarnation != current;
    if rebind {
        *kind = "owner-capability-rebind";
        if previous.generation >= generation
            || previous
                .runtime_identity
                .as_ref()
                .map(crate::coordination_runtime_status_for_identity)
                != Some(CoordinationRuntimeStatus::Stopped)
            || broker::heartbeat_fresh(context, id, &previous.incarnation, previous.heartbeat_epoch)
        {
            return Err(refused("owner-recovery-predecessor-unverified"));
        }
        if locked.registry.operations.iter().any(|operation| {
            operation.session_id == id
                && matches!(
                    operation.state.as_str(),
                    "active" | "completing" | "reconcile_pending"
                )
        }) {
            return Err(refused("owner-recovery-operation-pending"));
        }
        let active: Vec<_> = locked
            .registry
            .claims
            .iter()
            .filter(|claim| claim.session_id == id && claim.state == "active")
            .collect();
        if active.len() > 1
            || active
                .iter()
                .any(|claim| claim.session_incarnation != previous.incarnation)
        {
            return Err(refused("owner-recovery-claim-conflict"));
        }
    } else if previous.generation != generation
        || previous.runtime_identity_digest != runtime.identity_digest
    {
        return Err(refused("owner-recovery-binding-mismatch"));
    }
    for binding in [&current, &previous.incarnation] {
        crate::coordination::ensure_notification_submission_not_in_progress(
            &locked.registry,
            id,
            binding,
        )?;
    }
    let missing = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Ok(_) if !rebind => {
            let token = read_private_file(&path, 512)
                .map_err(|_| refused("owner-recovery-file-invalid"))?;
            if !digest_eq(&digest_bytes(&token), &previous.capability_digest) {
                return Err(refused("owner-recovery-file-invalid"));
            }
            false
        }
        _ => return Err(refused("owner-recovery-file-invalid")),
    };
    // Recheck the exact root after registry contention, immediately before authority writes.
    prove_caller(&record)?;
    let now = now_epoch();
    if missing {
        broker::prepare(context, &record)?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        // Stage a complete private file, commit its digest, then publish without
        // clobbering any existing entry. Interruption leaves either an absent
        // canonical path (safe to retry) or the complete matching capability.
        let mut staged = tempfile::Builder::new()
            .prefix(".owner-capability-")
            .tempfile_in(path.parent().expect("canonical coordination directory"))
            .map_err(|_| refused("owner-recovery-file-unavailable"))?;
        if staged
            .write_all(token.as_bytes())
            .and_then(|_| staged.as_file().sync_all())
            .is_err()
        {
            return Err(refused("owner-recovery-file-unavailable"));
        }
        let owner = locked.registry.brokers.get_mut(id).expect("locked broker");
        owner.incarnation = current.clone();
        owner.generation = generation;
        owner.runtime_identity = Some(runtime.identity.clone());
        owner.runtime_identity_digest = runtime.identity_digest.clone();
        owner.capability_digest = digest_bytes(token.as_bytes());
        owner.state = "recovering".into();
        owner.lost_since_epoch = Some(now);
        owner.heartbeat_epoch = 0;
        owner.heartbeat_at.clear();
        if rebind {
            for claim in &mut locked.registry.claims {
                if claim.session_id == id
                    && claim.session_incarnation == previous.incarnation
                    && claim.state == "active"
                {
                    claim.session_incarnation = current.clone();
                    claim.revision = claim.revision.saturating_add(1);
                    claim.updated_at = timestamp(now);
                }
            }
            crate::coordination::mailbox::carry_unread_after_verified_resume(
                context,
                &mut locked.registry,
                id,
                &record.created_at,
                &previous.incarnation,
                &current,
                now,
            );
        }
        if locked.save().is_err() {
            return Err(refused("owner-recovery-store-unavailable"));
        }
        staged
            .persist_noclobber(&path)
            .map_err(|_| refused("owner-recovery-file-unavailable"))?;
    } else {
        let owner = locked.registry.brokers.get_mut(id).expect("locked broker");
        owner.state = "recovering".into();
        owner.lost_since_epoch = Some(now);
        owner.heartbeat_epoch = 0;
        owner.heartbeat_at.clear();
        locked
            .save()
            .map_err(|_| refused("owner-recovery-store-unavailable"))?;
    }
    // Invalidate retained liveness only after the complete capability is published.
    // A failed startup stays recovering, so an immediate retry cannot authenticate
    // normally using the restored token and a pre-restoration heartbeat.
    match fs::remove_file(crate::coordination::heartbeat_path(&context.state_dir, id)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(heartbeat_unavailable()),
    }
    drop(locked);
    broker::spawn_heartbeat_sidecar(context, id, &current, generation, &path)
        .map_err(|_| heartbeat_unavailable())?;
    let started = Instant::now();
    while !broker::heartbeat_fresh(context, id, &current, 0) {
        if started.elapsed() >= Duration::from_secs(4) {
            return Err(heartbeat_unavailable());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut locked = lock_registry_observational(context)?;
    prove_caller(&record)?;
    let owner = locked
        .registry
        .brokers
        .get_mut(id)
        .filter(|owner| {
            owner.incarnation == current
                && owner.generation == generation
                && owner.runtime_identity_digest == runtime.identity_digest
                && owner.state == "recovering"
        })
        .ok_or_else(heartbeat_unavailable)?;
    owner.state = "ready".into();
    owner.heartbeat_epoch = now_epoch();
    owner.heartbeat_at = timestamp(owner.heartbeat_epoch);
    owner.lost_since_epoch = None;
    locked.save().map_err(|_| heartbeat_unavailable())?;
    drop(locked);
    broker::ensure_ready(context, &record).map_err(|_| heartbeat_unavailable())?;
    Ok((record, current))
}

#[cfg(target_os = "linux")]
fn prove_caller(record: &SessionRecord) -> Result<(), CliError> {
    let reject = || refused("owner-recovery-caller-unverified");
    let identity = crate::persisted_tmux_runtime_identity(record)
        .map_err(|_| reject())?
        .ok_or_else(reject)?;
    if crate::linux_runtime_pid_namespace_relation(&identity)
        != crate::LinuxPidNamespaceRelation::Same
    {
        return Err(reject());
    }
    let start = identity
        .resolved_pane_start_time()
        .map_err(|_| reject())?
        .ok_or_else(reject)?;
    // Linux /proc stat is kernel-owned. Parent links and birth times are bounded,
    // re-read evidence; neither a process group nor a launch string authenticates a peer.
    let mut pid = unsafe { libc::getpid() };
    for _ in 0..128 {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|_| reject())?;
        let fields: Vec<_> = stat
            .rsplit_once(") ")
            .ok_or_else(reject)?
            .1
            .split_whitespace()
            .collect();
        if fields.len() <= 19 || fields[0] == "Z" {
            return Err(reject());
        }
        let birth = fields[19].parse::<u64>().map_err(|_| reject())?;
        if pid == identity.pane_pid {
            if birth != start {
                return Err(reject());
            }
            let root = crate::read_linux_process_identity(pid)
                .map_err(|_| reject())?
                .ok_or_else(reject)?;
            return if root.start_time == start && !root.zombie {
                Ok(())
            } else {
                Err(reject())
            };
        }
        let parent = fields[1].parse::<libc::pid_t>().map_err(|_| reject())?;
        if parent <= 1 || parent == pid {
            return Err(reject());
        }
        pid = parent;
    }
    Err(reject())
}

#[cfg(not(target_os = "linux"))]
fn prove_caller(_record: &SessionRecord) -> Result<(), CliError> {
    // Current non-Linux launch records do not persist the pane birth identity.
    Err(refused("owner-recovery-platform-unverified"))
}
