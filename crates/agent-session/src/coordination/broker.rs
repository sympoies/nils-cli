use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::{BrokerHeartbeatArgs, BrokerRecoveryArgs, BrokerStatusArgs, BrokerStopArgs};
use crate::{CliContext, CliError, SessionRecord};

use super::{
    BrokerRecord, authenticate_from_file, authenticate_recovery_from_file, capability_path,
    checkpoint_path_for_state, clean_expired, digest_bytes, ensure_fingerprint_key,
    idempotency_replay, incarnation, json_value, lock_registry, now_epoch, read_bounded_json,
    request_digest, store_receipt, timestamp,
};

pub(crate) const BROKER_VERSION: &str = "agent-session.coordination-broker.v1";
pub(crate) const DSH_PROVIDER_LEASE_FAILURE_FILE: &str = ".dsh-provider-lease-failure";
const DSH_PROVIDER_LEASES_DIR: &str = "provider-session-leases";
const STARTUP_RUNTIME_CONFIRMATION_WINDOW: Duration = Duration::from_secs(5);
const STARTUP_RUNTIME_RETRY_INTERVAL: Duration = Duration::from_millis(20);
const STOPPED_RUNTIME_CONFIRMATION_INTERVAL: Duration = Duration::from_millis(250);

fn startup_runtime_retry_interval(status: crate::CoordinationRuntimeStatus) -> Duration {
    match status {
        crate::CoordinationRuntimeStatus::Stopped | crate::CoordinationRuntimeStatus::Unknown => {
            STOPPED_RUNTIME_CONFIRMATION_INTERVAL
        }
        crate::CoordinationRuntimeStatus::Running => STARTUP_RUNTIME_RETRY_INTERVAL,
    }
}

fn prepare_checkpoint_file(path: &Path) -> Result<bool, CliError> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(SECRET_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => {
            if file
                .set_permissions(fs::Permissions::from_mode(SECRET_FILE_MODE))
                .is_err()
            {
                let _ = fs::remove_file(path);
                return Err(unavailable());
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(|_| unavailable())?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
                || metadata.permissions().mode() & 0o777 != SECRET_FILE_MODE
            {
                return Err(CliError::runtime(
                    "coordination-store-untrusted",
                    "session checkpoint file is untrusted",
                    None,
                ));
            }
            Ok(false)
        }
        Err(_) => Err(unavailable()),
    }
}

#[derive(Clone, Debug, Serialize)]
struct BrokerStatus {
    schema_version: String,
    session_id: String,
    state: String,
    generation: u64,
    capability_available: bool,
    heartbeat_fresh: bool,
    claim: Option<ClaimSummary>,
    operation: OperationSummary,
}

#[derive(Clone, Debug, Serialize)]
struct ClaimSummary {
    claim_id: String,
    revision: u64,
    state: String,
    expires_at: String,
}

#[derive(Clone, Debug, Default, Serialize)]
struct OperationSummary {
    active: usize,
    uncertain: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecoveryProof {
    schema_version: String,
    session_incarnation: String,
    generation: u64,
}

pub(crate) fn prepare(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    prepare_in_dir(&crate::session_dir(context, &record.id))
}

pub(crate) fn prepare_in_dir(session_dir: &Path) -> Result<(), CliError> {
    let capability_dir = session_dir.join("coordination");
    match fs::symlink_metadata(&capability_dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(CliError::runtime(
                    "coordination-store-untrusted",
                    "session coordination credential directory is untrusted",
                    None,
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&capability_dir).map_err(|_| unavailable())?;
        }
        Err(_) => return Err(unavailable()),
    }
    fs::set_permissions(&capability_dir, fs::Permissions::from_mode(0o700))
        .map_err(|_| unavailable())
}

/// The old tmux target may be reused by the replacement, so retain its
/// pre-launch observation rather than probing that target after launch.
pub(crate) struct PreviousRuntimeEvidence {
    record: SessionRecord,
    status: crate::CoordinationRuntimeStatus,
    #[cfg(target_os = "macos")]
    stopped_broker_identity: Option<crate::TmuxRuntimeIdentity>,
}

pub(crate) fn capture_previous_runtime(
    record: &SessionRecord,
    tmux_bin: &Path,
) -> PreviousRuntimeEvidence {
    let status = incarnation(record)
        .ok()
        .map(|incarnation| legacy_runtime_status(record, &incarnation, tmux_bin))
        .unwrap_or(crate::CoordinationRuntimeStatus::Unknown);
    #[cfg(target_os = "macos")]
    let stopped_broker_identity = crate::persisted_tmux_runtime_identity(record)
        .ok()
        .flatten()
        .filter(|identity| {
            status == crate::CoordinationRuntimeStatus::Stopped
                && identity.macos_boot_id == crate::capture_macos_boot_id()
                && identity.macos_boot_id.is_some()
                && crate::verified_tmux_status_with_timeout(
                    tmux_bin,
                    &identity.session_id,
                    crate::DELETE_TERMINATION_VERIFY_TIMEOUT,
                ) == "stopped"
                && macos_process_group_is_empty(identity)
        });
    PreviousRuntimeEvidence {
        record: record.clone(),
        status,
        #[cfg(target_os = "macos")]
        stopped_broker_identity,
    }
}

/// Positive current-boot process evidence wins; Linux retains namespace checks.
/// Missing, invalid, or incarnation-mismatched persisted evidence stays unknown.
fn legacy_process_runtime_status(
    record: &SessionRecord,
    expected_incarnation: &str,
) -> crate::CoordinationRuntimeStatus {
    use crate::CoordinationRuntimeStatus::{Running, Stopped, Unknown};
    if incarnation(record).ok().as_deref() != Some(expected_incarnation) {
        return Unknown;
    }
    let Ok(Some(identity)) = crate::persisted_tmux_runtime_identity(record) else {
        return Unknown;
    };
    if crate::runtime_is_from_prior_boot(&identity) {
        return Stopped;
    }
    #[cfg(target_os = "linux")]
    let status = crate::coordination_process_runtime_status(&identity);
    #[cfg(not(target_os = "linux"))]
    let status = identity
        .process_group_id
        .map(crate::process_group_status)
        .unwrap_or(crate::ProcessGroupStatus::Unknown);
    match status {
        crate::ProcessGroupStatus::Running => Running,
        crate::ProcessGroupStatus::Stopped => Stopped,
        crate::ProcessGroupStatus::Unknown => Unknown,
    }
}

/// Prior-boot evidence proves absence; otherwise a stopped process boundary
/// also needs the exact managed tmux name absent.
/// This probe runs before any replacement is launched (or during stop retirement).
fn legacy_runtime_status(
    record: &SessionRecord,
    expected_incarnation: &str,
    tmux_bin: &Path,
) -> crate::CoordinationRuntimeStatus {
    use crate::CoordinationRuntimeStatus::{Running, Stopped, Unknown};
    let status = legacy_process_runtime_status(record, expected_incarnation);
    if status != Stopped {
        return status;
    }
    if crate::recorded_runtime_is_from_prior_boot(record) {
        return Stopped;
    }
    match crate::verified_tmux_status_with_timeout(
        tmux_bin,
        &format!("={}", record.tmux_session),
        crate::DELETE_TERMINATION_VERIFY_TIMEOUT,
    )
    .as_str()
    {
        "stopped" => Stopped,
        "running" => Running,
        _ => Unknown,
    }
}

// This additional proof is local to replacement/retirement. Generic macOS
// coordination evidence remains conservative when only a process group is absent.
#[cfg(target_os = "macos")]
fn stopped_macos_broker_matches_previous(
    context: &CliContext,
    broker: &BrokerRecord,
    prior: &PreviousRuntimeEvidence,
) -> bool {
    let Some(identity) = prior.stopped_broker_identity.as_ref() else {
        return false;
    };
    let Ok(Some(record_identity)) = crate::persisted_tmux_runtime_identity(&prior.record) else {
        return false;
    };
    let Some(broker_identity) = broker
        .runtime_identity
        .as_ref()
        .and_then(|value| serde_json::from_value::<crate::TmuxRuntimeIdentity>(value.clone()).ok())
    else {
        return false;
    };
    broker.state == "stopped"
        && broker.capability_digest.is_empty()
        && fs::symlink_metadata(capability_path(
            context,
            &broker.session_id,
            &broker.incarnation,
        ))
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        && broker.session_id == prior.record.id
        && incarnation(&prior.record).is_ok_and(|value| value == broker.incarnation)
        && prior
            .record
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.generation == broker.generation)
        && identity.launch_id.as_deref() == Some(broker.incarnation.as_str())
        && broker.runtime_identity.as_ref()
            == prior.record.extra.get(crate::DELETE_TMUX_IDENTITY_KEY)
        && broker_identity == *identity
        && record_identity == *identity
        && prior.status == crate::CoordinationRuntimeStatus::Stopped
        && macos_process_group_is_empty(identity)
}

#[cfg(target_os = "macos")]
fn macos_process_group_is_empty(identity: &crate::TmuxRuntimeIdentity) -> bool {
    macos_process_group_absence_probe(identity).is_ok()
}

#[cfg(target_os = "macos")]
fn macos_process_group_absence_probe(identity: &crate::TmuxRuntimeIdentity) -> Result<(), String> {
    let Some(group) = identity.process_group_id.filter(|group| *group > 1) else {
        return Err("process-group-id: missing or invalid".to_string());
    };
    if crate::process_group_status(group) != crate::ProcessGroupStatus::Stopped {
        return Err("signal-before-snapshot: ESRCH not observed".to_string());
    }
    let mut command = Command::new("/bin/ps");
    command.env("LC_ALL", "C").args(["-axo", "pid=,pgid="]);
    process_group_snapshot_is_empty(command, group)?;
    if crate::process_group_status(group) != crate::ProcessGroupStatus::Stopped {
        return Err("signal-after-snapshot: ESRCH not observed".to_string());
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
const PROCESS_GROUP_SNAPSHOT_MAX_BYTES: usize = 1024 * 1024;

#[cfg(any(target_os = "macos", test))]
fn process_group_snapshot_is_empty(command: Command, group: libc::pid_t) -> Result<(), String> {
    let output = crate::run_output_with_timeout_and_strict_cap(
        command,
        crate::DELETE_TERMINATION_VERIFY_TIMEOUT,
        PROCESS_GROUP_SNAPSHOT_MAX_BYTES,
    )
    .map_err(|error| {
        format!(
            "ps-execution: error_kind={:?}; byte_count=unavailable; overflow={}",
            error.kind(),
            error.kind() == std::io::ErrorKind::InvalidData
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "ps-exit: status={:?}; byte_count={}; overflow=false",
            output.status.code(),
            output.stdout.len()
        ));
    }
    if !process_group_absent_from_ps(&output.stdout, group) {
        return Err(format!(
            "ps-snapshot: absent group not proven; status=0; byte_count={}; overflow=false",
            output.stdout.len()
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn process_group_absent_from_ps(output: &[u8], group: libc::pid_t) -> bool {
    let Ok(output) = std::str::from_utf8(output) else {
        return false;
    };
    let mut rows = 0;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let Some(pid) = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
        else {
            return false;
        };
        let Some(pgid) = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
        else {
            return false;
        };
        if pid < 0 || pgid < 0 || fields.next().is_some() || pgid == group {
            return false;
        }
        rows += 1;
    }
    rows > 0
}

pub(crate) fn provision(context: &CliContext, record: &SessionRecord) -> Result<PathBuf, CliError> {
    provision_with_previous(context, record, None)
}

/// Resume retains the old record before the new pane identity overwrites it.
/// Brokers without a recorded identity use the snapshot; revoked macOS brokers require
/// their own identity to match the exact pre-launch stopped proof.
pub(crate) fn provision_with_previous(
    context: &CliContext,
    record: &SessionRecord,
    prior_record: Option<&PreviousRuntimeEvidence>,
) -> Result<PathBuf, CliError> {
    crate::orchestration::ensure_session_not_quarantined(context, record)?;
    prepare(context, record)?;
    let incarnation = incarnation(record)?;
    let runtime = crate::coordination_runtime_evidence(context, record).ok();
    let generation = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.generation)
        .unwrap_or_default();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let path = capability_path(context, &record.id, &incarnation);
    let now = now_epoch();
    let mut locked = lock_registry(context)?;
    // Close the race where provisioning passes the optimistic filesystem check
    // immediately before Main persists a session authority fence, then waits
    // for the coordination lock while Main seals the old broker.
    crate::orchestration::ensure_session_not_authority_quarantined(context, record)?;
    crate::orchestration::ensure_session_not_runtime_stop_fenced(context, record)?;
    let previous_broker = locked
        .registry
        .brokers
        .get(&record.id)
        .filter(|broker| broker.incarnation != incarnation)
        .cloned();
    #[cfg(target_os = "macos")]
    let mut used_stopped_macos_proof = false;
    if let Some(previous) = previous_broker.as_ref() {
        let heartbeat_live = heartbeat_fresh(
            context,
            &record.id,
            &previous.incarnation,
            previous.heartbeat_epoch,
        );
        #[allow(unused_mut)]
        let mut previous_runtime_status = previous
            .runtime_identity
            .as_ref()
            .map(crate::coordination_runtime_status_for_identity)
            .unwrap_or_else(|| {
                prior_record
                    .filter(|prior| {
                        prior.record.id == record.id
                            && prior
                                .record
                                .runtime
                                .as_ref()
                                .is_some_and(|runtime| runtime.generation == previous.generation)
                    })
                    .map(|prior| {
                        match legacy_process_runtime_status(&prior.record, &previous.incarnation) {
                            crate::CoordinationRuntimeStatus::Stopped => prior.status,
                            status => status,
                        }
                    })
                    .unwrap_or(crate::CoordinationRuntimeStatus::Unknown)
            });
        #[cfg(target_os = "macos")]
        if previous_runtime_status == crate::CoordinationRuntimeStatus::Unknown
            && prior_record.is_some_and(|prior| {
                stopped_macos_broker_matches_previous(context, previous, prior)
            })
        {
            previous_runtime_status = crate::CoordinationRuntimeStatus::Stopped;
            used_stopped_macos_proof = true;
        }
        if heartbeat_live || previous_runtime_status == crate::CoordinationRuntimeStatus::Running {
            if previous.lost_since_epoch.is_some() {
                if let Some(previous) = locked.registry.brokers.get_mut(&record.id) {
                    previous.lost_since_epoch = None;
                }
                locked.save()?;
            }
            return Err(CliError::data(
                "session-incarnation-conflict",
                "the prior coordination incarnation is still live",
                Some(
                    json!({"proof_step": if heartbeat_live { "heartbeat-fresh" } else { "process-group-probe" }}),
                ),
            ));
        }
        if previous_runtime_status == crate::CoordinationRuntimeStatus::Unknown {
            return Err(CliError::runtime(
                "coordination-runtime-unverified",
                "the prior coordination runtime identity cannot be proven stopped",
                Some(json!({"proof_step": "process-group-probe"})),
            ));
        }
        let previous_operation = locked.registry.operations.iter().any(|lease| {
            lease.session_id == record.id
                && lease.session_incarnation == previous.incarnation
                && matches!(
                    lease.state.as_str(),
                    "active" | "completing" | "reconcile_pending"
                )
        });
        if previous_operation {
            let unexpired = locked.registry.operations.iter().any(|lease| {
                lease.session_id == record.id
                    && lease.session_incarnation == previous.incarnation
                    && matches!(
                        lease.state.as_str(),
                        "active" | "completing" | "reconcile_pending"
                    )
                    && lease.expires_at_epoch > now
            });
            if unexpired {
                return Err(CliError::data(
                    "operation-in-progress",
                    "an unexpired operation remains bound to the prior incarnation",
                    None,
                ));
            }
            let lost_since = previous.lost_since_epoch.unwrap_or(now);
            if previous.lost_since_epoch.is_none() {
                if let Some(previous) = locked.registry.brokers.get_mut(&record.id) {
                    previous.lost_since_epoch = Some(now);
                }
                locked.save()?;
            }
            if lost_since > now.saturating_sub(10 * 60) {
                return Err(CliError::data(
                    "broker-replacement-grace",
                    "expired prior operations require ten minutes of continuously stopped runtime evidence",
                    Some(json!({ "retry_after_epoch": lost_since.saturating_add(10 * 60) })),
                ));
            }
        }
    } else if locked.registry.operations.iter().any(|lease| {
        lease.session_id == record.id
            && lease.session_incarnation != incarnation
            && matches!(
                lease.state.as_str(),
                "active" | "completing" | "reconcile_pending"
            )
    }) {
        return Err(CliError::data(
            "operation-in-progress",
            "an unresolved prior operation has no terminal runtime evidence",
            None,
        ));
    }
    let previous_capability = previous_broker
        .as_ref()
        .map(|broker| capability_path(context, &record.id, &broker.incarnation));
    let previous_checkpoint = previous_broker.as_ref().map(|broker| {
        checkpoint_path_for_state(&context.state_dir, &record.id, &broker.incarnation)
    });
    for claim in locked.registry.claims.iter().filter(|claim| {
        claim.session_id == record.id
            && claim.session_incarnation != incarnation
            && claim.state == "active"
    }) {
        super::ensure_claim_mutation_not_fenced(
            context,
            &record.id,
            &claim.session_incarnation,
            claim,
        )?;
    }
    #[cfg(target_os = "macos")]
    if used_stopped_macos_proof
        && !previous_broker
            .as_ref()
            .zip(prior_record)
            .is_some_and(|(previous, prior)| {
                stopped_macos_broker_matches_previous(context, previous, prior)
            })
    {
        return Err(CliError::runtime(
            "coordination-runtime-unverified",
            "the prior coordination runtime failed the macOS stopped process-boundary recheck",
            None,
        ));
    }
    write_atomic(&path, token.as_bytes(), SECRET_FILE_MODE).map_err(|_| unavailable())?;
    let checkpoint_path = checkpoint_path_for_state(&context.state_dir, &record.id, &incarnation);
    let checkpoint_created = match prepare_checkpoint_file(&checkpoint_path) {
        Ok(created) => created,
        Err(error) => {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
    };
    ensure_fingerprint_key(&mut locked.registry);
    clean_expired(&mut locked.registry, now);
    for claim in &mut locked.registry.claims {
        if claim.session_id == record.id
            && claim.session_incarnation != incarnation
            && claim.state == "active"
        {
            claim.state = "released".to_string();
            claim.revision = claim.revision.saturating_add(1);
            claim.updated_at = timestamp(now);
            claim.terminal_at_epoch = Some(now);
        }
    }
    for lease in &mut locked.registry.operations {
        if lease.session_id == record.id
            && lease.session_incarnation != incarnation
            && matches!(
                lease.state.as_str(),
                "active" | "completing" | "reconcile_pending"
            )
        {
            lease.state = "abandoned".to_string();
            lease.revision = lease.revision.saturating_add(1);
            lease.terminal_at_epoch = Some(now);
        }
    }
    if let Some(previous) = previous_broker.as_ref() {
        // Persisted with the replacement broker below, so nothing can still be
        // admitted for the predecessor once its unread mail has moved.
        super::mailbox::carry_unread_after_verified_resume(
            context,
            &mut locked.registry,
            &record.id,
            &previous.incarnation,
            &incarnation,
            now,
        );
    }
    locked.registry.brokers.insert(
        record.id.clone(),
        BrokerRecord {
            session_id: record.id.clone(),
            incarnation,
            coordination_mode: record.coordination_mode,
            capability_digest: digest_bytes(token.as_bytes()),
            generation,
            state: "starting".to_string(),
            heartbeat_at: String::new(),
            heartbeat_epoch: 0,
            runtime_identity: runtime.as_ref().map(|runtime| runtime.identity.clone()),
            runtime_identity_digest: runtime
                .as_ref()
                .map(|runtime| runtime.identity_digest.clone())
                .unwrap_or_default(),
            lost_since_epoch: None,
            binary_version: Some(super::broker_binary_version()),
        },
    );
    if let Err(error) = locked.save() {
        let _ = fs::remove_file(&path);
        if checkpoint_created {
            let _ = fs::remove_file(&checkpoint_path);
        }
        return Err(error);
    }
    if let Some(previous) = previous_capability {
        let _ = fs::remove_file(previous);
    }
    if let Some(previous) = previous_checkpoint {
        let _ = fs::remove_file(previous);
    }
    Ok(path)
}

pub(crate) fn activate_ready(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    let incarnation = incarnation(record)?;
    let _runtime = crate::coordination_runtime_evidence(context, record)?;
    let started = Instant::now();
    while !heartbeat_fresh(context, &record.id, &incarnation, 0) {
        if dsh_provider_lease_conflict_path(context, record).is_file() {
            return Err(provider_session_already_running());
        }
        if started.elapsed() >= Duration::from_secs(2) {
            return Err(CliError::runtime(
                "coordination-broker-start-timeout",
                "the identity-bound coordination heartbeat did not become ready",
                None,
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    let path = capability_path(context, &record.id, &incarnation);
    let token = read_private_capability(&path)?;
    let now = now_epoch();
    let mut locked = lock_registry(context)?;
    let broker = locked
        .registry
        .brokers
        .get_mut(&record.id)
        .filter(|broker| {
            broker.incarnation == incarnation
                && broker.state == "starting"
                && super::digest_eq(
                    &broker.capability_digest,
                    &digest_bytes(token.trim().as_bytes()),
                )
        })
        .ok_or_else(unavailable)?;
    broker.state = "ready".to_string();
    broker.heartbeat_at = timestamp(now);
    broker.heartbeat_epoch = now;
    locked.save()?;
    let _ = fs::remove_file(dsh_provider_lease_conflict_path(context, record));
    Ok(())
}

pub(crate) fn ensure_ready(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    let incarnation = incarnation(record)?;
    let token = read_private_capability(&capability_path(context, &record.id, &incarnation))?;
    let locked = lock_registry(context)?;
    locked
        .registry
        .brokers
        .get(&record.id)
        .filter(|broker| {
            broker.incarnation == incarnation
                && broker.state == "ready"
                && super::digest_eq(
                    &broker.capability_digest,
                    &digest_bytes(token.trim().as_bytes()),
                )
                && heartbeat_fresh(context, &record.id, &incarnation, broker.heartbeat_epoch)
        })
        .ok_or_else(unavailable)?;
    Ok(())
}

fn revoke_locked(
    context: &CliContext,
    record: &SessionRecord,
    enforce_runtime_stop_fence: bool,
) -> Result<(), CliError> {
    let now = now_epoch();
    let current_incarnation = incarnation(record).ok();
    let mut locked = lock_registry(context)?;
    if enforce_runtime_stop_fence {
        crate::orchestration::ensure_session_not_runtime_stop_fenced(context, record)?;
    }
    for claim in locked.registry.claims.iter().filter(|claim| {
        claim.session_id == record.id
            && current_incarnation
                .as_deref()
                .is_some_and(|current| current == claim.session_incarnation)
            && claim.state == "active"
    }) {
        super::ensure_claim_mutation_not_fenced(
            context,
            &record.id,
            &claim.session_incarnation,
            claim,
        )?;
    }
    if let Some(broker) = locked.registry.brokers.get_mut(&record.id)
        && current_incarnation
            .as_deref()
            .is_some_and(|current| current == broker.incarnation)
    {
        broker.state = "stopped".to_string();
        broker.heartbeat_at = timestamp(now);
        broker.heartbeat_epoch = now;
        broker.capability_digest.clear();
    }
    for claim in &mut locked.registry.claims {
        if claim.session_id == record.id
            && current_incarnation
                .as_deref()
                .is_some_and(|current| current == claim.session_incarnation)
            && claim.state == "active"
        {
            claim.state = "released".to_string();
            claim.revision = claim.revision.saturating_add(1);
            claim.updated_at = timestamp(now);
            claim.terminal_at_epoch = Some(now);
        }
    }
    for operation in &mut locked.registry.operations {
        if operation.session_id == record.id
            && current_incarnation
                .as_deref()
                .is_some_and(|current| current == operation.session_incarnation)
            && matches!(
                operation.state.as_str(),
                "active" | "completing" | "reconcile_pending"
            )
        {
            operation.state = "abandoned".to_string();
            operation.revision = operation.revision.saturating_add(1);
            operation.terminal_at_epoch = Some(now);
        }
    }
    if let Some(current) = current_incarnation.as_deref() {
        remove_advisory_state_for_incarnation(&mut locked.registry, &record.id, current);
    }
    locked.save()?;
    if let Some(current) = current_incarnation.as_deref() {
        let _ = fs::remove_file(capability_path(context, &record.id, current));
        let _ = fs::remove_file(checkpoint_path_for_state(
            &context.state_dir,
            &record.id,
            current,
        ));
    }
    Ok(())
}

pub(crate) fn revoke(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    revoke_locked(context, record, false)
}

/// Retire the coordination incarnation of a runtime the caller just verified
/// stopped, as its launch wrapper's `broker stop` would have had it exited on
/// its own. The heartbeat writer dies with that runtime, but its last beat stays
/// fresh for the freshness window and would refuse the session's own resume as
/// a live prior incarnation, so it is expired too. The heartbeat is touched only
/// while the registry still proves this exact incarnation's runtime stopped.
pub(crate) fn retire_after_verified_stop(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
) -> Result<(), CliError> {
    let incarnation = incarnation(record)?;
    let previous_runtime = capture_previous_runtime(record, tmux_bin);
    revoke(context, record)?;
    let locked = lock_registry(context)?;
    let proven_stopped = locked
        .registry
        .brokers
        .get(&record.id)
        .filter(|broker| broker.incarnation == incarnation)
        .is_some_and(|broker| {
            #[cfg(target_os = "macos")]
            if stopped_macos_broker_matches_previous(context, broker, &previous_runtime) {
                return true;
            }
            broker
                .runtime_identity
                .as_ref()
                .map(crate::coordination_runtime_status_for_identity)
                .unwrap_or_else(|| {
                    match legacy_process_runtime_status(&previous_runtime.record, &incarnation) {
                        crate::CoordinationRuntimeStatus::Stopped => previous_runtime.status,
                        status => status,
                    }
                })
                == crate::CoordinationRuntimeStatus::Stopped
        });
    if proven_stopped && heartbeat_fresh(context, &record.id, &incarnation, 0) {
        match fs::remove_file(super::heartbeat_path(&context.state_dir, &record.id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable()),
        }
    }
    drop(locked);
    Ok(())
}

pub(crate) fn forget_revoked_failed_launch(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    let incarnation = incarnation(record)?;
    let mut locked = lock_registry(context)?;
    let removable = locked
        .registry
        .brokers
        .get(&record.id)
        .is_some_and(|broker| {
            broker.incarnation == incarnation
                && broker.state == "stopped"
                && broker.capability_digest.is_empty()
        })
        && !locked.registry.claims.iter().any(|claim| {
            claim.session_id == record.id
                && claim.session_incarnation == incarnation
                && claim.state == "active"
        })
        && !locked.registry.operations.iter().any(|operation| {
            operation.session_id == record.id
                && operation.session_incarnation == incarnation
                && matches!(
                    operation.state.as_str(),
                    "active" | "completing" | "reconcile_pending"
                )
        });
    if removable {
        locked.registry.brokers.remove(&record.id);
        locked.save()?;
    }
    Ok(())
}

fn revoke_unless_runtime_stop_fenced(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    revoke_locked(context, record, true)
}

fn pause_broker_stop_for_test() -> Result<(), CliError> {
    #[cfg(debug_assertions)]
    if let Some(directory) =
        std::env::var_os("NILS_AGENT_SESSION_TEST_BROKER_STOP_BARRIER_DIR").map(PathBuf::from)
    {
        fs::create_dir_all(&directory).map_err(|_| unavailable())?;
        fs::write(directory.join("ready"), b"after_fence_check").map_err(|_| unavailable())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !directory.join("release").is_file() {
            if Instant::now() >= deadline {
                return Err(CliError::runtime(
                    "test-barrier-timeout",
                    "broker stop test barrier timed out",
                    None,
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

fn remove_advisory_state_for_incarnation(
    registry: &mut super::Registry,
    session_id: &str,
    session_incarnation: &str,
) {
    if registry
        .advisory_acknowledgements
        .get(session_id)
        .is_some_and(|acknowledgement| acknowledgement.session_incarnation == session_incarnation)
    {
        registry.advisory_acknowledgements.remove(session_id);
    }
    if registry
        .advisory_observations
        .get(session_id)
        .is_some_and(|observation| observation.session_incarnation == session_incarnation)
    {
        registry.advisory_observations.remove(session_id);
    }
}

pub(crate) fn stop(context: &CliContext, args: BrokerStopArgs) -> Result<Value, CliError> {
    let journal_id = args.session.clone();
    let exit_code = args.exit_code;
    let before = crate::load_session_record(context, &journal_id).ok();
    let result = crate::lifecycle::attempt(context, &journal_id, "broker-stop", || {
        let (record, _) =
            authenticate_from_file(context, &args.session, args.capability_file.as_deref())?;
        crate::orchestration::ensure_session_not_runtime_stop_fenced(context, &record)?;
        pause_broker_stop_for_test()?;
        revoke_unless_runtime_stop_fenced(context, &record)?;
        Ok(json!({
            "schema_version": BROKER_VERSION,
            "session_id": record.id,
            "state": "stopped"
        }))
    });
    if let Some(code) = exit_code {
        crate::lifecycle::record(
            context,
            before.as_ref(),
            &journal_id,
            "exit-observed",
            "controller",
            result.as_ref().map(|_| ()),
            Some(
                json!({"code": code, "signal": null, "reason": "runtime-exited", "stopped_by": null}),
            ),
        );
    }
    result
}

pub(crate) fn status(context: &CliContext, args: BrokerStatusArgs) -> Result<Value, CliError> {
    let authenticated_incarnation = if args.authenticated {
        Some(authenticate_from_file(context, &args.session, args.capability_file.as_deref())?.1)
    } else {
        None
    };
    let locked = lock_registry(context)?;
    let broker = locked
        .registry
        .brokers
        .get(&args.session)
        .filter(|broker| {
            authenticated_incarnation
                .as_ref()
                .is_none_or(|incarnation| broker.incarnation == *incarnation)
        })
        .ok_or_else(unavailable)?;
    let incarnation = broker.incarnation.clone();
    let claim = locked
        .registry
        .claims
        .iter()
        .find(|claim| {
            claim.session_id == args.session
                && claim.session_incarnation == incarnation
                && claim.state == "active"
        })
        .map(|claim| ClaimSummary {
            claim_id: claim.claim_id.clone(),
            revision: claim.revision,
            state: claim.state.clone(),
            expires_at: claim.expires_at.clone(),
        });
    let operation = OperationSummary {
        active: locked
            .registry
            .operations
            .iter()
            .filter(|lease| {
                lease.session_id == args.session
                    && lease.session_incarnation == incarnation
                    && lease.state == "active"
            })
            .count(),
        uncertain: locked
            .registry
            .operations
            .iter()
            .filter(|lease| {
                lease.session_id == args.session
                    && lease.session_incarnation == incarnation
                    && matches!(lease.state.as_str(), "completing" | "reconcile_pending")
            })
            .count(),
    };
    json_value(BrokerStatus {
        schema_version: BROKER_VERSION.to_string(),
        session_id: args.session.clone(),
        state: broker.state.clone(),
        generation: broker.generation,
        capability_available: capability_available(
            context,
            &args.session,
            &incarnation,
            &broker.capability_digest,
        ),
        heartbeat_fresh: heartbeat_fresh(
            context,
            &args.session,
            &incarnation,
            broker.heartbeat_epoch,
        ),
        claim,
        operation,
    })
}

pub(crate) fn recover(
    context: &CliContext,
    args: BrokerRecoveryArgs,
    reconcile: bool,
) -> Result<Value, CliError> {
    let journal_id = args.session.clone();
    crate::lifecycle::attempt(
        context,
        &journal_id,
        if reconcile {
            "broker-reconcile"
        } else {
            "broker-adopt"
        },
        || recover_unjournaled(context, args, reconcile),
    )
}

pub(crate) fn recover_unjournaled(
    context: &CliContext,
    args: BrokerRecoveryArgs,
    reconcile: bool,
) -> Result<Value, CliError> {
    let (authenticated_record, authenticated_incarnation) =
        authenticate_recovery_from_file(context, &args.session, args.capability_file.as_deref())?;
    let proof: RecoveryProof =
        read_bounded_json(&args.proof_file, 8 * 1024, "invalid-recovery-proof")?;
    if proof.schema_version != "agent-session.coordination-recovery-proof.v1" {
        return Err(CliError::data(
            "invalid-recovery-proof",
            "recovery proof schema is unsupported",
            None,
        ));
    }
    let _session_lock = crate::acquire_session_record_lock(context, &args.session)?;
    let record = crate::load_session_record(context, &args.session)?;
    crate::ensure_same_session_identity(&authenticated_record, &record)?;
    let record_incarnation = incarnation(&record)?;
    if record_incarnation != authenticated_incarnation {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "the authenticated recovery capability no longer matches the current runtime",
            None,
        ));
    }
    let record_generation = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.generation)
        .unwrap_or_default();
    if proof.session_incarnation != record_incarnation || proof.generation != record_generation {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "recovery proof does not match the current runtime",
            None,
        ));
    }
    let operation = if reconcile {
        "broker-reconcile"
    } else {
        "broker-adopt"
    };
    let reconcile_selector = if reconcile {
        if !args.attest_inactive {
            return Err(CliError::usage(
                "invalid-recovery-proof",
                "broker reconcile requires --attest-inactive",
                None,
            ));
        }
        Some((
            args.operation.as_deref().ok_or_else(|| {
                CliError::usage(
                    "invalid-recovery-proof",
                    "broker reconcile requires --operation",
                    None,
                )
            })?,
            args.if_revision.ok_or_else(|| {
                CliError::usage(
                    "invalid-recovery-proof",
                    "broker reconcile requires --if-revision",
                    None,
                )
            })?,
        ))
    } else {
        if args.operation.is_some() || args.if_revision.is_some() || args.attest_inactive {
            return Err(CliError::usage(
                "invalid-recovery-proof",
                "broker adopt does not accept operation reconciliation selectors",
                None,
            ));
        }
        None
    };
    let digest = request_digest(
        operation,
        &json!({
            "proof": proof,
            "operation": args.operation,
            "if_revision": args.if_revision,
            "attest_inactive": args.attest_inactive,
        }),
    );
    {
        let locked = lock_registry(context)?;
        if let Some(replay) = idempotency_replay(
            &locked.registry,
            &args.idempotency_key,
            &record.id,
            &record_incarnation,
            operation,
            &digest,
        )? {
            return Ok(replay);
        }
        if locked
            .registry
            .brokers
            .get(&record.id)
            .is_some_and(|broker| {
                broker.incarnation == record_incarnation
                    && broker.state == "ready"
                    && capability_available(
                        context,
                        &record.id,
                        &record_incarnation,
                        &broker.capability_digest,
                    )
                    && heartbeat_fresh(
                        context,
                        &record.id,
                        &record_incarnation,
                        broker.heartbeat_epoch,
                    )
            })
        {
            return Err(CliError::data(
                "coordination-broker-not-lost",
                "the exact coordination broker is still healthy",
                None,
            ));
        }
    }
    let runtime = crate::coordination_runtime_evidence(context, &record)?;
    if runtime.status != crate::CoordinationRuntimeStatus::Running {
        return Err(CliError::runtime(
            "coordination-runtime-unverified",
            "broker recovery requires the exact persisted runtime to be running",
            None,
        ));
    }
    let mut locked = lock_registry(context)?;
    if let Some(replay) = idempotency_replay(
        &locked.registry,
        &args.idempotency_key,
        &record.id,
        &record_incarnation,
        operation,
        &digest,
    )? {
        return Ok(replay);
    }
    let broker_snapshot = locked
        .registry
        .brokers
        .get(&record.id)
        .filter(|broker| {
            broker.incarnation == record_incarnation && broker.generation == record_generation
        })
        .cloned()
        .ok_or_else(unavailable)?;
    if broker_snapshot.state == "ready"
        && capability_available(
            context,
            &record.id,
            &record_incarnation,
            &broker_snapshot.capability_digest,
        )
        && heartbeat_fresh(
            context,
            &record.id,
            &record_incarnation,
            broker_snapshot.heartbeat_epoch,
        )
    {
        return Err(CliError::data(
            "coordination-broker-not-lost",
            "the exact coordination broker is still healthy",
            None,
        ));
    }
    if broker_snapshot.runtime_identity_digest != runtime.identity_digest
        || !capability_available(
            context,
            &record.id,
            &record_incarnation,
            &broker_snapshot.capability_digest,
        )
    {
        return Err(CliError::runtime(
            "coordination-runtime-unverified",
            "recovery evidence does not match the persisted broker identity and capability",
            None,
        ));
    }
    let sidecar_already_running = broker_snapshot.state == "recovering"
        && heartbeat_fresh(context, &record.id, &record_incarnation, 0);
    if broker_snapshot.state != "recovering" {
        let broker = locked
            .registry
            .brokers
            .get_mut(&record.id)
            .filter(|broker| {
                broker.incarnation == record_incarnation
                    && broker.generation == record_generation
                    && broker.runtime_identity_digest == runtime.identity_digest
            })
            .ok_or_else(unavailable)?;
        broker.state = "recovering".to_string();
        broker.lost_since_epoch.get_or_insert(now_epoch());
        locked.save()?;
    }
    drop(locked);
    let heartbeat = super::heartbeat_path(&context.state_dir, &record.id);
    let heartbeat_before = fs::metadata(&heartbeat)
        .and_then(|metadata| metadata.modified())
        .ok();
    if !sidecar_already_running {
        match fs::remove_file(&heartbeat) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable()),
        }
    }
    spawn_heartbeat_sidecar(
        context,
        &record.id,
        &record_incarnation,
        record_generation,
        &capability_path(context, &record.id, &record_incarnation),
    )?;
    let started = Instant::now();
    while !heartbeat_fresh(context, &record.id, &record_incarnation, 0)
        || !heartbeat_advanced_since(&heartbeat, heartbeat_before)
    {
        if started.elapsed() >= Duration::from_secs(4) {
            return Err(CliError::runtime(
                "coordination-broker-start-timeout",
                "recovered broker sidecar did not become ready",
                None,
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    let runtime = crate::coordination_runtime_evidence(context, &record)?;
    if runtime.status != crate::CoordinationRuntimeStatus::Running
        || runtime.identity_digest != broker_snapshot.runtime_identity_digest
    {
        return Err(CliError::runtime(
            "coordination-runtime-unverified",
            "the recovered runtime changed before broker commit",
            None,
        ));
    }
    let _activity_fence = if reconcile {
        Some(crate::activity::acquire_coordination_activity_lock(
            context, &record.id,
        )?)
    } else {
        None
    };
    let now = now_epoch();
    let mut locked = lock_registry(context)?;
    if let Some(replay) = idempotency_replay(
        &locked.registry,
        &args.idempotency_key,
        &record.id,
        &record_incarnation,
        operation,
        &digest,
    )? {
        return Ok(replay);
    }
    let broker = locked
        .registry
        .brokers
        .get_mut(&record.id)
        .filter(|broker| {
            broker.incarnation == record_incarnation
                && broker.generation == record_generation
                && broker.state == "recovering"
                && broker.runtime_identity_digest == runtime.identity_digest
        })
        .ok_or_else(unavailable)?;
    broker.state = "ready".to_string();
    broker.heartbeat_at = timestamp(now);
    broker.heartbeat_epoch = now;
    broker.lost_since_epoch = None;
    let operation_reconciliation = reconcile_selector
        .map(|(lease_id, revision)| {
            super::claims::operator_reconcile_in_registry(
                context,
                &mut locked.registry,
                &record,
                lease_id,
                revision,
                now,
            )
        })
        .transpose()?;
    let result = json!({
        "schema_version": BROKER_VERSION,
        "session_id": record.id,
        "state": "ready",
        "generation": record_generation,
        "recovery": if reconcile { "reconciled" } else { "adopted" },
        "operation_reconciliation": operation_reconciliation,
    });
    store_receipt(
        &mut locked.registry,
        args.idempotency_key,
        record.id,
        record_incarnation,
        operation.to_string(),
        digest,
        result.clone(),
        now,
    )?;
    locked.save()?;
    Ok(result)
}

fn spawn_heartbeat_sidecar(
    context: &CliContext,
    session_id: &str,
    incarnation: &str,
    generation: u64,
    capability_file: &std::path::Path,
) -> Result<(), CliError> {
    let executable = crate::resolve_agent_session_executable().map_err(|_| unavailable())?;
    let mut command = Command::new(executable);
    command
        .arg("--state-dir")
        .arg(&context.state_dir)
        .arg("broker")
        .arg("heartbeat")
        .arg("--session")
        .arg(session_id)
        .arg("--incarnation")
        .arg(incarnation)
        .arg("--generation")
        .arg(generation.to_string())
        .arg("--capability-file")
        .arg(capability_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the child performs only async-signal-safe `setsid` before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    command.spawn().map_err(|_| unavailable())?;
    Ok(())
}

pub(crate) fn run_heartbeat_sidecar(
    context: &CliContext,
    args: BrokerHeartbeatArgs,
) -> Result<Value, CliError> {
    if heartbeat_owner_authorization(context, &args) == HeartbeatAuthorization::Revoked {
        return Err(super::unauthorized());
    }
    let directory = super::coordination_dir(context, &args.session);
    let lock_path = directory.join(format!(
        "broker-{}.lock",
        digest_bytes(args.incarnation.as_bytes())
    ));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(SECRET_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(lock_path)
        .map_err(|_| unavailable())?;
    use std::os::fd::AsRawFd;
    // SAFETY: `lock` owns a valid descriptor for the lifetime of the loop.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(CliError::data(
            "coordination-broker-not-lost",
            "an exact broker heartbeat owner already exists",
            None,
        ));
    }
    let started = Instant::now();
    let mut established_owner = false;
    let mut observed_stopped = false;
    let mut activated_external_broker = false;
    let mut provider_session_lease = None;
    loop {
        let record = match crate::load_session_record(context, &args.session) {
            Ok(record) => record,
            Err(_) if started.elapsed() < Duration::from_secs(2) => {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(_) => break,
        };
        let matches = incarnation(&record).is_ok_and(|value| value == args.incarnation)
            && record
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.generation == args.generation);
        if !matches {
            break;
        }
        match heartbeat_owner_authorization(context, &args) {
            HeartbeatAuthorization::Authorized => {}
            HeartbeatAuthorization::Revoked => break,
            HeartbeatAuthorization::Unknown => {
                // Skip this beat rather than claim liveness we could not check.
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        }
        if provider_session_lease.is_none() {
            match acquire_dsh_provider_session_lease(context, &record) {
                Ok(lease) => provider_session_lease = lease,
                Err(error) if error.code() == "provider-session-already-running" => {
                    let _ = write_atomic(
                        &dsh_provider_lease_conflict_path(context, &record),
                        b"provider-session-already-running\n",
                        SECRET_FILE_MODE,
                    );
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
        established_owner = true;
        match crate::coordination_runtime_evidence(context, &record) {
            Ok(runtime) if runtime.status == crate::CoordinationRuntimeStatus::Running => {}
            Ok(runtime)
                if runtime.status == crate::CoordinationRuntimeStatus::Stopped
                    && started.elapsed() >= STARTUP_RUNTIME_CONFIRMATION_WINDOW =>
            {
                observed_stopped = true;
                crate::lifecycle::observe_stopped(context, &record);
                break;
            }
            Ok(runtime) if started.elapsed() < STARTUP_RUNTIME_CONFIRMATION_WINDOW => {
                thread::sleep(startup_runtime_retry_interval(runtime.status));
                continue;
            }
            Err(_) if started.elapsed() < STARTUP_RUNTIME_CONFIRMATION_WINDOW => {
                thread::sleep(STARTUP_RUNTIME_RETRY_INTERVAL);
                continue;
            }
            Ok(_) | Err(_) => {
                mark_degraded(context, &args);
                break;
            }
        }
        let _ = super::claims::drain_completion_events(context);
        let now = now_epoch();
        write_atomic(
            &super::heartbeat_path(&context.state_dir, &args.session),
            format!("{}:{}\n", args.incarnation, now).as_bytes(),
            SECRET_FILE_MODE,
        )
        .map_err(|_| unavailable())?;
        // An external-runtime lane has no launcher of ours to activate its
        // broker. `main-agent worker start` can only provision it: at that
        // point neither the lane's runtime evidence nor this heartbeat exists,
        // and `activate_ready` requires both. The first live beat is therefore
        // the earliest moment readiness is provable, and without activating
        // here the broker stays `starting` forever — every authenticated worker
        // call, starting with `main-agent bootstrap`, fails
        // `coordination-unauthorized`. Tmux launches keep their existing
        // launcher-driven activation and never reach this branch.
        if !activated_external_broker && crate::dsh_external::is_external_record(&record) {
            activated_external_broker = activate_ready(context, &record).is_ok()
        }
        thread::sleep(Duration::from_secs(2));
    }
    if established_owner
        && observed_stopped
        && let Ok(record) = crate::load_session_record(context, &args.session)
        && incarnation(&record).is_ok_and(|value| value == args.incarnation)
        && record
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.generation == args.generation)
        && crate::orchestration::ensure_session_not_runtime_stop_fenced(context, &record).is_ok()
    {
        let _ = revoke_unless_runtime_stop_fenced(context, &record);
    }
    Ok(json!({
        "schema_version": BROKER_VERSION,
        "session_id": args.session,
        "state": "stopped"
    }))
}

fn dsh_provider_lease_conflict_path(context: &CliContext, record: &SessionRecord) -> PathBuf {
    crate::session_dir(context, &record.id).join(DSH_PROVIDER_LEASE_FAILURE_FILE)
}

fn acquire_dsh_provider_session_lease(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<Option<fs::File>, CliError> {
    let Some(scope) = crate::dsh_provider_lease_scope(record)? else {
        return Ok(None);
    };
    acquire_provider_session_lease_scope(context, &scope).map(Some)
}

fn acquire_provider_session_lease_scope(
    context: &CliContext,
    scope: &str,
) -> Result<fs::File, CliError> {
    let root = super::coordination_root(context)?;
    let lease_root = root.join(DSH_PROVIDER_LEASES_DIR);
    match fs::symlink_metadata(&lease_root) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() } =>
        {
            return Err(CliError::runtime(
                "coordination-store-untrusted",
                "provider session lease directory is untrusted",
                None,
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(error) = fs::create_dir(&lease_root)
                && error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(unavailable());
            }
            let metadata = fs::symlink_metadata(&lease_root).map_err(|_| unavailable())?;
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(CliError::runtime(
                    "coordination-store-untrusted",
                    "provider session lease directory is untrusted",
                    None,
                ));
            }
        }
        Err(_) => return Err(unavailable()),
    }
    fs::set_permissions(&lease_root, fs::Permissions::from_mode(0o700))
        .map_err(|_| unavailable())?;
    let path = lease_root.join(format!("{}.lock", digest_bytes(scope.as_bytes())));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(SECRET_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| unavailable())?;
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(CliError::runtime(
            "coordination-store-untrusted",
            "provider session lease file is untrusted",
            None,
        ));
    }
    use std::os::fd::AsRawFd;
    // SAFETY: `file` owns a valid descriptor retained by the heartbeat sidecar
    // for the managed runtime's full lifetime.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Err(provider_session_already_running());
        }
        return Err(unavailable());
    }
    Ok(file)
}

fn provider_session_already_running() -> CliError {
    CliError::data(
        "provider-session-already-running",
        "the DSH provider session already has a live managed writer",
        Some(json!({
            "retryable": true,
            "next_action": "stop_existing_session"
        })),
    )
}

fn heartbeat_advanced_since(
    heartbeat: &std::path::Path,
    previous: Option<std::time::SystemTime>,
) -> bool {
    let Ok(current) = fs::metadata(heartbeat).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    previous.is_none_or(|previous| current > previous)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeartbeatAuthorization {
    Authorized,
    Revoked,
    /// The registry could not be read, typically because the lock is busy.
    /// That is contention, not revocation, so the sidecar skips this beat and
    /// retries instead of exiting (sympoies/nils-cli#1860).
    Unknown,
}

// Read-only: an observational lock runs no claim or lease maintenance and
// never rewrites the registry, so a 2 s beat does not add write load.
fn heartbeat_owner_authorization(
    context: &CliContext,
    args: &BrokerHeartbeatArgs,
) -> HeartbeatAuthorization {
    let Ok(token) = read_private_capability(&args.capability_file) else {
        return HeartbeatAuthorization::Revoked;
    };
    let Ok(locked) = super::lock_registry_observational(context) else {
        return HeartbeatAuthorization::Unknown;
    };
    let authorized = locked
        .registry
        .brokers
        .get(&args.session)
        .is_some_and(|broker| {
            broker.incarnation == args.incarnation
                && broker.generation == args.generation
                && matches!(broker.state.as_str(), "starting" | "recovering" | "ready")
                && super::digest_eq(
                    &broker.capability_digest,
                    &digest_bytes(token.trim().as_bytes()),
                )
        });
    if authorized {
        HeartbeatAuthorization::Authorized
    } else {
        HeartbeatAuthorization::Revoked
    }
}

fn mark_degraded(context: &CliContext, args: &BrokerHeartbeatArgs) {
    let Ok(mut locked) = lock_registry(context) else {
        return;
    };
    let Some(broker) = locked.registry.brokers.get_mut(&args.session) else {
        return;
    };
    if broker.incarnation != args.incarnation || broker.generation != args.generation {
        return;
    }
    let newly_lost = broker.state != "degraded";
    broker.state = "degraded".to_string();
    broker.lost_since_epoch.get_or_insert(now_epoch());
    let saved = locked.save().is_ok();
    drop(locked);
    if newly_lost && saved {
        let target = crate::load_session_record(context, &args.session).ok();
        let error = unavailable();
        crate::lifecycle::record(
            context,
            target.as_ref(),
            &args.session,
            "broker-heartbeat-loss",
            "controller",
            Err(&error),
            None,
        );
    }
}

fn unavailable() -> CliError {
    CliError::runtime(
        "coordination-broker-lost",
        "coordination broker is unavailable",
        None,
    )
}

fn read_private_capability(path: &std::path::Path) -> Result<String, CliError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| unavailable())?;
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file()
        || metadata.len() > 512
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(CliError::runtime(
            "coordination-store-untrusted",
            "coordination capability file is untrusted",
            None,
        ));
    }
    let mut token = String::new();
    file.by_ref()
        .take(513)
        .read_to_string(&mut token)
        .map_err(|_| unavailable())?;
    if token.len() > 512 {
        return Err(unavailable());
    }
    Ok(token)
}

pub(crate) fn capability_available(
    context: &CliContext,
    session_id: &str,
    incarnation: &str,
    expected_digest: &str,
) -> bool {
    if expected_digest.is_empty() {
        return false;
    }
    read_private_capability(&capability_path(context, session_id, incarnation)).is_ok_and(|token| {
        super::digest_eq(expected_digest, &digest_bytes(token.trim().as_bytes()))
    })
}

pub(crate) fn heartbeat_fresh(
    context: &CliContext,
    session_id: &str,
    incarnation: &str,
    _registry_heartbeat_epoch: i64,
) -> bool {
    heartbeat_fresh_with_clock(context, session_id, incarnation, now_epoch)
}

fn heartbeat_fresh_with_clock(
    context: &CliContext,
    session_id: &str,
    incarnation: &str,
    clock: impl FnOnce() -> i64,
) -> bool {
    nils_common::coordination_projection::heartbeat_fresh_with_clock(
        &context.state_dir,
        session_id,
        incarnation,
        clock,
    )
}

/// Seeds for tests outside this module that exercise a stopped runtime's
/// coordination incarnation.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use serde_json::json;
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::CommandExt;

    /// A process group that has exited and been reaped.
    pub(crate) fn exited_process_group() -> libc::pid_t {
        let mut exited = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .process_group(0)
            .spawn()
            .expect("spawn");
        let group = exited.id() as libc::pid_t;
        exited.wait().expect("reap");
        group
    }

    /// A stopped-boundary fixture verified by the real platform probes.
    pub(crate) fn verified_absent_process_group() -> libc::pid_t {
        let group = exited_process_group();
        assert_eq!(
            crate::process_group_status(group),
            crate::ProcessGroupStatus::Stopped,
            "fixture process-group signal probe must report ESRCH: group={group}"
        );
        #[cfg(target_os = "macos")]
        {
            let identity =
                serde_json::from_value::<crate::TmuxRuntimeIdentity>(process_group_identity(group))
                    .unwrap();
            super::macos_process_group_absence_probe(&identity).expect(
                "fixture complete PID/PGID enumeration and signal recheck must prove absence",
            );
        }
        group
    }

    /// Runtime identity evidence for `process_group` in this pid namespace.
    pub(crate) fn process_group_identity(process_group: libc::pid_t) -> Value {
        #[cfg(target_os = "linux")]
        let namespace = fs::metadata("/proc/self/ns/pid").expect("pid namespace");
        #[cfg(target_os = "linux")]
        let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id").expect("boot id");
        let identity = json!({
            "session_id": "$1",
            "pane_id": "%1",
            "pane_pid": process_group,
            "process_group_id": process_group,
        });
        #[cfg(target_os = "linux")]
        let identity = {
            let mut identity = identity;
            identity["pid_namespace"] = json!({
                "device": namespace.dev(),
                "inode": namespace.ino(),
                "boot_id": boot_id.trim(),
            });
            identity
        };
        identity
    }

    pub(crate) fn make_legacy_broker(context: &CliContext, session_id: &str, fresh: bool) {
        let mut locked = lock_registry(context).expect("registry");
        let broker = locked.registry.brokers.get_mut(session_id).expect("broker");
        broker.runtime_identity = None;
        broker.heartbeat_epoch = 0;
        locked.save().expect("save older broker");
        if !fresh {
            fs::remove_file(super::super::heartbeat_path(&context.state_dir, session_id)).unwrap();
        }
    }

    /// A ready broker for `incarnation` with a fresh heartbeat, as a running
    /// launch leaves it the moment its runtime is killed.
    pub(crate) fn seed_live_broker(
        context: &CliContext,
        session_id: &str,
        incarnation: &str,
        runtime_identity: Value,
    ) {
        let mut locked = lock_registry(context).expect("registry");
        locked.registry.brokers.insert(
            session_id.to_string(),
            super::super::BrokerRecord {
                session_id: session_id.to_string(),
                incarnation: incarnation.to_string(),
                coordination_mode: Default::default(),
                capability_digest: "digest".to_string(),
                generation: 1,
                state: "ready".to_string(),
                heartbeat_at: String::new(),
                heartbeat_epoch: now_epoch(),
                runtime_identity: Some(runtime_identity),
                runtime_identity_digest: String::new(),
                lost_since_epoch: None,
                binary_version: None,
            },
        );
        locked.save().expect("seed broker");
        drop(locked);
        let heartbeat = super::super::heartbeat_path(&context.state_dir, session_id);
        fs::create_dir_all(heartbeat.parent().unwrap()).unwrap();
        fs::write(&heartbeat, format!("{incarnation}:{}\n", now_epoch())).unwrap();
        fs::set_permissions(&heartbeat, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::json;
    use std::os::unix::fs::symlink;

    #[test]
    fn controller_loss_records_one_exact_runtime_failure() {
        use pretty_assertions::assert_eq;
        let dir = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: dir.path().to_path_buf(),
            host: None,
        };
        let record = seed_retirable_broker(&context, json!({}));
        crate::write_session_record(&context, &record).unwrap();
        let args = BrokerHeartbeatArgs {
            session: "session".into(),
            incarnation: "old".into(),
            generation: 1,
            capability_file: dir.path().join("unused-capability"),
            format: nils_common::cli_contract::OutputFormat::Json,
        };
        mark_degraded(&context, &args);
        mark_degraded(&context, &args);
        let journal = crate::lifecycle::read(&context, "session", 10).unwrap();
        let rows = journal["records"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["operation"], "broker-heartbeat-loss");
        assert_eq!(rows[0]["session_incarnation"], "old");
        assert_eq!(rows[0]["session_generation"], 1);
        assert_eq!(rows[0]["result"]["code"], "coordination-broker-lost");
        assert_eq!(rows[0]["result"]["proof_step"], "broker-state");
    }

    #[test]
    fn heartbeat_clock_boundary_does_not_report_a_healthy_broker_lost() {
        let temporary = tempfile::TempDir::new().expect("temporary state");
        let context = CliContext {
            state_dir: temporary.path().to_path_buf(),
            host: None,
        };
        let heartbeat =
            nils_common::coordination_projection::heartbeat_path(&context.state_dir, "worker");
        fs::create_dir_all(heartbeat.parent().unwrap()).unwrap();
        fs::write(&heartbeat, "worker-inc:100\n").unwrap();
        fs::set_permissions(&heartbeat, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            heartbeat_fresh_with_clock(&context, "worker", "worker-inc", || {
                // Publish the next second exactly where the reader samples its clock.
                let replacement = heartbeat.with_extension("next");
                fs::write(&replacement, "worker-inc:101\n").unwrap();
                fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
                fs::rename(replacement, &heartbeat).unwrap();
                100
            }),
            "an update after the clock sample must not falsely lose a healthy broker"
        );
    }

    #[test]
    fn provider_session_lease_rejects_a_second_writer_and_releases_on_owner_drop() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        fs::create_dir_all(&context.state_dir).unwrap();
        let first = acquire_provider_session_lease_scope(&context, "dsh\0/root\0session")
            .expect("first writer owns the lease");
        let conflict = acquire_provider_session_lease_scope(&context, "dsh\0/root\0session")
            .expect_err("second writer must be rejected");
        assert_eq!(conflict.code(), "provider-session-already-running");
        let other = acquire_provider_session_lease_scope(&context, "dsh\0/root\0other-session")
            .expect("another provider identity has an independent lease");

        drop(first);
        acquire_provider_session_lease_scope(&context, "dsh\0/root\0session")
            .expect("kernel-released ownership is stale-safe and immediately reusable");
        drop(other);
    }

    #[test]
    fn checkpoint_file_is_private_and_reuses_only_a_trusted_inode() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("checkpoint.json");

        assert!(prepare_checkpoint_file(&path).expect("create checkpoint"));
        let metadata = fs::symlink_metadata(&path).expect("checkpoint metadata");
        assert!(metadata.is_file());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(metadata.permissions().mode() & 0o777, SECRET_FILE_MODE);

        fs::write(&path, b"{\"state\":\"working\"}\n").expect("seed checkpoint");
        assert!(!prepare_checkpoint_file(&path).expect("reuse trusted checkpoint"));
        assert_eq!(
            fs::read(&path).expect("read checkpoint"),
            b"{\"state\":\"working\"}\n"
        );
    }

    #[test]
    fn checkpoint_file_rejects_public_symlink_and_hardlink_targets() {
        let tmp = tempfile::TempDir::new().expect("tempdir");

        let public = tmp.path().join("public.json");
        fs::write(&public, b"public").expect("seed public");
        fs::set_permissions(&public, fs::Permissions::from_mode(0o644)).expect("set public mode");
        let error = prepare_checkpoint_file(&public).expect_err("public file rejected");
        assert_eq!(error.code(), "coordination-store-untrusted");
        assert_eq!(fs::read(&public).expect("public unchanged"), b"public");

        let sentinel = tmp.path().join("sentinel");
        fs::write(&sentinel, b"sentinel").expect("seed sentinel");
        fs::set_permissions(&sentinel, fs::Permissions::from_mode(SECRET_FILE_MODE))
            .expect("set private mode");
        let linked = tmp.path().join("linked.json");
        fs::hard_link(&sentinel, &linked).expect("hard link");
        let error = prepare_checkpoint_file(&linked).expect_err("hard link rejected");
        assert_eq!(error.code(), "coordination-store-untrusted");
        assert_eq!(
            fs::read(&sentinel).expect("sentinel unchanged"),
            b"sentinel"
        );

        let symbolic = tmp.path().join("symbolic.json");
        symlink(&sentinel, &symbolic).expect("symlink");
        let error = prepare_checkpoint_file(&symbolic).expect_err("symlink rejected");
        assert_eq!(error.code(), "coordination-store-untrusted");
        assert_eq!(
            fs::read(&sentinel).expect("sentinel unchanged"),
            b"sentinel"
        );
    }

    #[test]
    fn broker_projection_schema_is_stable() {
        let value = serde_json::to_value(BrokerStatus {
            schema_version: BROKER_VERSION.to_string(),
            session_id: "session".to_string(),
            state: "ready".to_string(),
            generation: 1,
            capability_available: true,
            heartbeat_fresh: true,
            claim: None,
            operation: OperationSummary::default(),
        })
        .expect("serialize");
        assert!(value.get("capability_path").is_none());
        assert!(value.get("capability_digest").is_none());
    }

    #[test]
    fn stopped_runtime_confirmation_bounds_process_snapshot_polling() {
        assert_eq!(
            startup_runtime_retry_interval(crate::CoordinationRuntimeStatus::Stopped),
            STOPPED_RUNTIME_CONFIRMATION_INTERVAL
        );
        assert!(
            STARTUP_RUNTIME_CONFIRMATION_WINDOW.as_millis()
                / STOPPED_RUNTIME_CONFIRMATION_INTERVAL.as_millis()
                <= 20,
            "the stopped confirmation window must not trigger hundreds of process snapshots"
        );
        assert_eq!(
            startup_runtime_retry_interval(crate::CoordinationRuntimeStatus::Unknown),
            STOPPED_RUNTIME_CONFIRMATION_INTERVAL
        );
    }

    #[test]
    fn coordination_review_recent_registry_timestamp_is_not_readiness() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        assert!(!heartbeat_fresh(
            &context,
            "session",
            "incarnation",
            now_epoch()
        ));
    }

    #[test]
    fn coordination_review_recovery_proof_rejects_raw_operator_tokens() {
        let value = json!({
            "schema_version": "agent-session.coordination-recovery-proof.v1",
            "session_incarnation": "incarnation",
            "generation": 1,
            "operator_token": "raw-secret"
        });
        assert!(serde_json::from_value::<RecoveryProof>(value).is_err());
    }

    #[test]
    fn coordination_review_heartbeat_requires_private_launch_authority() {
        let missing = crate::cli::Cli::try_parse_from([
            "agent-session",
            "broker",
            "heartbeat",
            "--session",
            "session",
            "--incarnation",
            "incarnation",
            "--generation",
            "1",
        ]);
        assert!(missing.is_err());
        assert!(
            crate::cli::Cli::try_parse_from([
                "agent-session",
                "broker",
                "heartbeat",
                "--session",
                "session",
                "--incarnation",
                "incarnation",
                "--generation",
                "1",
                "--capability-file",
                "/private/capability",
            ])
            .is_ok()
        );
    }

    #[test]
    fn stale_revoke_preserves_replacement_advisory_state() {
        let mut registry = super::super::Registry::default();
        registry.advisory_acknowledgements.insert(
            "session".to_string(),
            crate::coordination::advisory::AdvisoryAcknowledgement {
                session_incarnation: "replacement".to_string(),
                advisory_digest: "digest".to_string(),
                expires_at: "2030-01-01T00:00:00Z".to_string(),
                expires_at_epoch: i64::MAX,
            },
        );
        registry.advisory_observations.insert(
            "session".to_string(),
            crate::coordination::advisory::AdvisoryObservation {
                session_incarnation: "replacement".to_string(),
                advisory_digest: "digest".to_string(),
                observed_at_epoch: i64::MAX,
            },
        );

        remove_advisory_state_for_incarnation(&mut registry, "session", "stale");
        assert!(registry.advisory_acknowledgements.contains_key("session"));
        assert!(registry.advisory_observations.contains_key("session"));

        remove_advisory_state_for_incarnation(&mut registry, "session", "replacement");
        assert!(!registry.advisory_acknowledgements.contains_key("session"));
        assert!(!registry.advisory_observations.contains_key("session"));
    }

    fn seed_retirable_broker(context: &CliContext, runtime_identity: Value) -> SessionRecord {
        let record: SessionRecord = serde_json::from_value(json!({
            "schema_version": crate::SESSION_DOCUMENT_VERSION,
            "id": "session",
            "agent": "claude",
            "mode": "interactive",
            "title": "Switching",
            "cwd": "/srv/outside-home",
            "tmux_session": "agent-session",
            "prompt_file": null,
            "log_file": null,
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z",
            "runtime": {
                "kind": "tmux",
                "tmux_session": "agent-session",
                "generation": 1,
                "started_at": "2030-01-01T00:00:00Z",
                "launch_id": "old"
            }
        }))
        .expect("record");
        test_support::seed_live_broker(context, "session", "old", runtime_identity);
        record
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn generation_two_legacy_broker_capture_ignores_recycled_numeric_selector() {
        use pretty_assertions::assert_eq;
        for prior_boot in [true, false] {
            let temporary = tempfile::TempDir::new().unwrap();
            let context = CliContext {
                state_dir: temporary.path().to_path_buf(),
                host: None,
            };
            let tmux = temporary.path().join("tmux");
            fs::write(
                &tmux,
                "#!/bin/sh\nif [ \"$3\" = '$7' ]; then exit 0; fi\nprintf '%s\\n' \"can't find session: fixture\" >&2\nexit 1\n",
            )
            .unwrap();
            fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
            let identity =
                test_support::process_group_identity(test_support::exited_process_group());
            let mut prior = seed_retirable_broker(&context, identity.clone());
            prior.runtime.as_mut().unwrap().generation = 2;
            let mut identity = identity;
            identity["launch_id"] = json!("old");
            identity["session_id"] = json!("$7");
            if prior_boot {
                let boot = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
                assert_ne!(crate::linux_boot_id().unwrap(), boot);
                identity["pid_namespace"]["boot_id"] = json!(boot);
            }
            prior
                .extra
                .insert("delete_tmux_identity".to_string(), identity);
            crate::write_session_record(&context, &prior).unwrap();
            test_support::make_legacy_broker(&context, &prior.id, false);
            let mut locked = lock_registry(&context).unwrap();
            locked
                .registry
                .brokers
                .get_mut(&prior.id)
                .unwrap()
                .generation = 2;
            locked.save().unwrap();
            drop(locked);

            let captured = capture_previous_runtime(&prior, &tmux);
            assert_eq!(captured.status, crate::CoordinationRuntimeStatus::Stopped);
            let mut replacement = prior.clone();
            replacement.runtime.as_mut().unwrap().generation = 3;
            replacement.runtime.as_mut().unwrap().launch_id = "new".to_string();
            provision_with_previous(&context, &replacement, Some(&captured)).unwrap();
        }
    }

    #[test]
    fn legacy_replacement_keeps_live_unknown_and_broker_owned_evidence_fenced() {
        use pretty_assertions::assert_eq;
        let temporary = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: temporary.path().to_path_buf(),
            host: None,
        };
        let tmux = temporary.path().join("tmux");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf '%s\n' \"can't find session: fixture\" >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let stopped_identity =
            test_support::process_group_identity(test_support::exited_process_group());
        let mut prior = seed_retirable_broker(&context, stopped_identity.clone());
        let mut persisted = stopped_identity.clone();
        persisted["launch_id"] = json!("old");
        prior
            .extra
            .insert("delete_tmux_identity".to_string(), persisted);
        crate::write_session_record(&context, &prior).unwrap();
        let mut replacement = prior.clone();
        replacement.runtime.as_mut().unwrap().launch_id = "new".to_string();
        replacement.runtime.as_mut().unwrap().generation = 2;

        // A missing registry identity can use only a proven stopped old record.
        test_support::make_legacy_broker(&context, "session", false);
        let captured = crate::persisted_tmux_runtime_identity(&prior)
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::process_group_status(captured.process_group_id.unwrap()),
            crate::ProcessGroupStatus::Stopped
        );
        assert_eq!(
            crate::verified_tmux_status_with_timeout(
                &tmux,
                &captured.session_id,
                Duration::from_secs(1)
            ),
            "stopped"
        );
        assert_eq!(
            legacy_runtime_status(&prior, "old", &tmux),
            crate::CoordinationRuntimeStatus::Stopped
        );
        provision_with_previous(
            &context,
            &replacement,
            Some(&capture_previous_runtime(&prior, &tmux)),
        )
        .unwrap();

        // Even stopped process evidence cannot override a fresh old heartbeat.
        test_support::seed_live_broker(&context, "session", "old", stopped_identity.clone());
        test_support::make_legacy_broker(&context, "session", true);
        assert_eq!(
            provision_with_previous(
                &context,
                &replacement,
                Some(&capture_previous_runtime(&prior, &tmux))
            )
            .unwrap_err()
            .code(),
            "session-incarnation-conflict"
        );
        test_support::make_legacy_broker(&context, "session", false);

        let mut live = prior.clone();
        let mut identity = test_support::process_group_identity(unsafe { libc::getpgrp() });
        identity["launch_id"] = json!("old");
        live.extra
            .insert("delete_tmux_identity".to_string(), identity);
        assert_eq!(
            provision_with_previous(
                &context,
                &replacement,
                Some(&capture_previous_runtime(&live, &tmux))
            )
            .unwrap_err()
            .code(),
            "session-incarnation-conflict"
        );

        for mutation in [
            "missing",
            "invalid",
            "wrong-launch",
            "wrong-session",
            "wrong-generation",
            "no-group",
        ] {
            let mut unverified = prior.clone();
            match mutation {
                "missing" => {
                    unverified.extra.remove("delete_tmux_identity");
                }
                "invalid" => {
                    unverified
                        .extra
                        .insert("delete_tmux_identity".to_string(), json!({}));
                }
                "wrong-launch" => {
                    unverified.extra.get_mut("delete_tmux_identity").unwrap()["launch_id"] =
                        json!("other");
                }
                "wrong-session" => {
                    unverified.id = "other".to_string();
                }
                "wrong-generation" => {
                    unverified.runtime.as_mut().unwrap().generation = 9;
                }
                "no-group" => {
                    unverified
                        .extra
                        .get_mut("delete_tmux_identity")
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("process_group_id");
                }
                _ => unreachable!(),
            }
            assert_eq!(
                provision_with_previous(
                    &context,
                    &replacement,
                    Some(&capture_previous_runtime(&unverified, &tmux))
                )
                .unwrap_err()
                .code(),
                "coordination-runtime-unverified",
                "{mutation}"
            );
        }
        fs::write(&tmux, "#!/bin/sh\nexit 0\n").unwrap();
        assert_eq!(
            provision_with_previous(
                &context,
                &replacement,
                Some(&capture_previous_runtime(&prior, &tmux))
            )
            .unwrap_err()
            .code(),
            "session-incarnation-conflict"
        );
        fs::write(&tmux, "#!/bin/sh\nexit 2\n").unwrap();
        assert_eq!(
            provision_with_previous(
                &context,
                &replacement,
                Some(&capture_previous_runtime(&prior, &tmux))
            )
            .unwrap_err()
            .code(),
            "coordination-runtime-unverified"
        );

        // A broker's own unknown evidence must never be replaced by the fallback.
        test_support::seed_live_broker(&context, "session", "old", json!({}));
        fs::remove_file(super::super::heartbeat_path(&context.state_dir, "session")).unwrap();
        assert_eq!(
            provision_with_previous(
                &context,
                &replacement,
                Some(&capture_previous_runtime(&prior, &tmux))
            )
            .unwrap_err()
            .code(),
            "coordination-runtime-unverified"
        );
    }

    #[test]
    fn stopped_broker_process_group_requires_complete_empty_enumeration() {
        assert!(process_group_absent_from_ps(b" 1 1\n 2 1\n 3 3\n", 7));
        for output in [
            b"".as_slice(),
            b"1 1\n2 7\n",
            b"1 1\n2 invalid\n",
            b"1 1\n2\n",
            b"1 1 extra\n",
            b"1 -1\n",
            b"\xff",
        ] {
            assert!(!process_group_absent_from_ps(output, 7), "{output:?}");
        }
    }

    #[test]
    fn stopped_broker_process_group_snapshot_rejects_a_truncated_prefix() {
        let temporary = tempfile::TempDir::new().unwrap();
        let path = temporary.path().join("snapshot");
        // The old 4 KiB cap ends exactly between rows, hiding a live member.
        fs::write(&path, format!("{}2 7\n", "1 1\n".repeat(1024))).unwrap();
        let mut command = Command::new("cat");
        command.arg(path);
        let error = process_group_snapshot_is_empty(command, 7).unwrap_err();
        assert!(error.starts_with("ps-snapshot:"), "{error}");
    }

    #[test]
    fn stopped_broker_process_group_snapshot_rejects_overflow() {
        let temporary = tempfile::TempDir::new().unwrap();
        let path = temporary.path().join("snapshot");
        fs::write(
            &path,
            "1 1\n".repeat(PROCESS_GROUP_SNAPSHOT_MAX_BYTES / 4 + 1),
        )
        .unwrap();
        let mut command = Command::new("cat");
        command.arg(path);
        let error = process_group_snapshot_is_empty(command, 7).unwrap_err();
        assert!(error.starts_with("ps-execution:"), "{error}");
        assert!(error.contains("overflow=true"), "{error}");
    }

    #[test]
    fn stopped_broker_process_group_snapshot_accepts_complete_output_at_the_cap() {
        let temporary = tempfile::TempDir::new().unwrap();
        let path = temporary.path().join("snapshot");
        fs::write(&path, "1 1\n".repeat(PROCESS_GROUP_SNAPSHOT_MAX_BYTES / 4)).unwrap();
        let mut command = Command::new("cat");
        command.arg(path);
        process_group_snapshot_is_empty(command, 7).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn same_boot_stopped_broker_retires_and_replaces_the_exact_runtime() {
        use pretty_assertions::assert_eq;
        let temporary = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: temporary.path().join("state"),
            host: None,
        };
        let tmux = temporary.path().join("tmux");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf '%s\\n' \"can't find session: fixture\" >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let mut identity =
            test_support::process_group_identity(test_support::verified_absent_process_group());
        identity["macos_boot_id"] = json!(crate::capture_macos_boot_id().unwrap());
        identity["launch_id"] = json!("old");
        let mut prior = seed_retirable_broker(&context, identity.clone());
        prior
            .extra
            .insert("delete_tmux_identity".to_string(), identity);
        crate::write_session_record(&context, &prior).unwrap();
        assert!(heartbeat_fresh(&context, "session", "old", 0));
        retire_after_verified_stop(&context, &prior, &tmux).unwrap();
        assert!(!heartbeat_fresh(&context, "session", "old", 0));
        let captured = capture_previous_runtime(&prior, &tmux);
        let mut replacement = prior.clone();
        let runtime = replacement.runtime.as_mut().unwrap();
        runtime.launch_id = "replacement".to_string();
        runtime.generation = 2;
        assert!(provision_with_previous(&context, &replacement, Some(&captured)).is_ok());
        assert_eq!(
            lock_registry(&context).unwrap().registry.brokers["session"].generation,
            2
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn same_boot_stopped_broker_refuses_unverified_or_mismatched_replacement() {
        use pretty_assertions::assert_eq;
        for mutation in [
            "identity",
            "missing-boot",
            "invalid-boot",
            "extra-field",
            "null-field",
            "session",
            "incarnation",
            "generation",
            "capability-file",
            "capability-digest",
            "ready-state",
            "managed-live",
            "numeric-live",
            "tmux-unknown",
            "live-group",
            "fresh-heartbeat",
        ] {
            let temporary = tempfile::TempDir::new().unwrap();
            let context = CliContext {
                state_dir: temporary.path().join("state"),
                host: None,
            };
            let tmux = temporary.path().join("tmux");
            fs::write(
                &tmux,
                "#!/bin/sh\nprintf '%s\\n' \"can't find session: fixture\" >&2\nexit 1\n",
            )
            .unwrap();
            fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
            let group = if mutation == "live-group" {
                unsafe { libc::getpgrp() }
            } else {
                test_support::verified_absent_process_group()
            };
            let mut identity = test_support::process_group_identity(group);
            identity["macos_boot_id"] = json!(crate::capture_macos_boot_id().unwrap());
            identity["launch_id"] = json!("old");
            if mutation == "missing-boot" {
                identity.as_object_mut().unwrap().remove("macos_boot_id");
            } else if mutation == "invalid-boot" {
                identity["macos_boot_id"] = json!("invalid");
            }
            let mut prior = seed_retirable_broker(&context, identity.clone());
            prior
                .extra
                .insert("delete_tmux_identity".to_string(), identity);
            crate::write_session_record(&context, &prior).unwrap();
            revoke(&context, &prior).unwrap();
            if mutation != "fresh-heartbeat" {
                fs::remove_file(super::super::heartbeat_path(&context.state_dir, &prior.id))
                    .unwrap();
            }
            if mutation == "managed-live" {
                fs::write(&tmux, "#!/bin/sh\n[ \"$3\" = '=agent-session' ] && exit 0\nprintf '%s\\n' \"can't find session: fixture\" >&2\nexit 1\n").unwrap();
            } else if mutation == "numeric-live" {
                fs::write(&tmux, "#!/bin/sh\n[ \"$3\" = '$1' ] && exit 0\nprintf '%s\\n' \"can't find session: fixture\" >&2\nexit 1\n").unwrap();
            } else if mutation == "tmux-unknown" {
                fs::write(&tmux, "#!/bin/sh\nexit 2\n").unwrap();
            }
            let mut captured = capture_previous_runtime(&prior, &tmux);
            {
                let mut locked = lock_registry(&context).unwrap();
                let broker = locked.registry.brokers.get_mut(&prior.id).unwrap();
                match mutation {
                    "identity" => {
                        broker.runtime_identity.as_mut().unwrap()["pane_id"] = json!("%2")
                    }
                    "extra-field" => {
                        broker.runtime_identity.as_mut().unwrap()["future_boundary"] =
                            json!("unverified")
                    }
                    "null-field" => {
                        broker.runtime_identity.as_mut().unwrap()["pane_start_time"] = Value::Null
                    }
                    "session" => captured.record.id = "other".to_string(),
                    "incarnation" => {
                        captured.record.runtime.as_mut().unwrap().launch_id = "other".to_string()
                    }
                    "generation" => broker.generation = 9,
                    "capability-digest" => broker.capability_digest = "retained".to_string(),
                    "ready-state" => broker.state = "ready".to_string(),
                    "capability-file" => {
                        prepare(&context, &prior).unwrap();
                        fs::write(capability_path(&context, &prior.id, "old"), b"retained")
                            .unwrap();
                    }
                    _ => {}
                }
                locked.save().unwrap();
            }
            let mut replacement = prior.clone();
            replacement.runtime.as_mut().unwrap().launch_id = "replacement".to_string();
            replacement.runtime.as_mut().unwrap().generation = 2;
            let error =
                provision_with_previous(&context, &replacement, Some(&captured)).unwrap_err();
            let expected = if matches!(mutation, "live-group" | "fresh-heartbeat") {
                "session-incarnation-conflict"
            } else {
                "coordination-runtime-unverified"
            };
            assert_eq!(error.code(), expected, "{mutation}");
            assert_eq!(
                lock_registry(&context).unwrap().registry.brokers[&prior.id].incarnation,
                "old",
                "{mutation}"
            );
        }
    }

    /// A verified stop kills the heartbeat writer with the runtime, but its last
    /// beat stays fresh for the freshness window and would refuse the session's
    /// own resume as "the prior coordination incarnation is still live".
    #[cfg(target_os = "linux")]
    #[test]
    fn retire_after_verified_stop_expires_only_a_proven_stopped_incarnation() {
        let temporary = tempfile::TempDir::new().expect("temporary state");
        let context = CliContext {
            state_dir: temporary.path().join("state"),
            host: None,
        };
        let record = seed_retirable_broker(
            &context,
            test_support::process_group_identity(test_support::exited_process_group()),
        );
        assert!(heartbeat_fresh(&context, "session", "old", 0));

        retire_after_verified_stop(&context, &record, Path::new("tmux")).expect("retire");

        assert!(
            !heartbeat_fresh(&context, "session", "old", 0),
            "a proven-stopped incarnation must not keep refusing the resume"
        );
        let locked = lock_registry(&context).expect("registry");
        assert_eq!(locked.registry.brokers["session"].state, "stopped");
        drop(locked);

        // A runtime that is still running keeps its liveness evidence.
        let own_group = unsafe { libc::getpgrp() };
        let record =
            seed_retirable_broker(&context, test_support::process_group_identity(own_group));
        retire_after_verified_stop(&context, &record, Path::new("tmux"))
            .expect("retire is a no-op");
        assert!(heartbeat_fresh(&context, "session", "old", 0));
    }

    #[test]
    fn heartbeat_authorization_treats_a_busy_registry_lock_as_unknown_not_revoked() {
        let temporary = tempfile::TempDir::new().expect("temporary state");
        let context = CliContext {
            state_dir: temporary.path().join("state"),
            host: None,
        };
        let token = "heartbeat-capability-token";
        {
            let mut locked = lock_registry(&context).expect("registry");
            locked.registry.brokers.insert(
                "session".to_string(),
                super::super::BrokerRecord {
                    session_id: "session".to_string(),
                    incarnation: "inc".to_string(),
                    coordination_mode: Default::default(),
                    capability_digest: digest_bytes(token.as_bytes()),
                    generation: 1,
                    state: "ready".to_string(),
                    heartbeat_at: String::new(),
                    heartbeat_epoch: 0,
                    runtime_identity: None,
                    runtime_identity_digest: String::new(),
                    lost_since_epoch: None,
                    binary_version: None,
                },
            );
            locked.save().expect("seed broker");
        }
        let capability_file = temporary.path().join("capability");
        fs::write(&capability_file, token).expect("capability");
        fs::set_permissions(&capability_file, fs::Permissions::from_mode(0o600))
            .expect("private capability");
        let args = BrokerHeartbeatArgs {
            session: "session".to_string(),
            incarnation: "inc".to_string(),
            generation: 1,
            capability_file,
            format: nils_common::cli_contract::OutputFormat::Json,
        };
        assert_eq!(
            heartbeat_owner_authorization(&context, &args),
            HeartbeatAuthorization::Authorized
        );

        // Another process holding the registry past the lock timeout is contention,
        // not revocation: the sidecar must keep its beat loop alive.
        let held = super::super::lock_private_store(&context, "registry.lock").expect("hold lock");
        assert_eq!(
            heartbeat_owner_authorization(&context, &args),
            HeartbeatAuthorization::Unknown
        );
        drop(held);

        {
            let mut locked = lock_registry(&context).expect("registry");
            locked
                .registry
                .brokers
                .get_mut("session")
                .expect("broker")
                .state = "stopped".to_string();
            locked.save().expect("stop broker");
        }
        assert_eq!(
            heartbeat_owner_authorization(&context, &args),
            HeartbeatAuthorization::Revoked
        );
    }
}
