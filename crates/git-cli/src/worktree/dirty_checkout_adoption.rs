use super::{CliError, detect_format, emit_error, emit_success, take_format};
use anyhow::{Context, Result, bail, ensure};
use nils_common::cli_contract::{OutputFormat, exit};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(any(test, target_os = "macos"))]
use std::collections::HashMap;
use std::collections::{HashSet, VecDeque};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const SNAPSHOT_SCHEMA: &str = "agent-runtime.dirty-checkout-snapshot.v1";
const CHALLENGE_SCHEMA: &str = "agent-runtime.dirty-checkout-challenge.v1";
const RECEIPT_SCHEMA: &str = "agent-runtime.dirty-checkout-receipt.v1";
const LEASE_V1_SCHEMA: &str = "agent-runtime.checkout-lease.v1";
const LEASE_V2_SCHEMA: &str = "agent-runtime.checkout-lease.v2";
const ADOPTION_SCHEMA: &str = "agent-runtime.dirty-checkout-adoption.v1";
const PENDING_ADOPTION_SCHEMA: &str = "agent-runtime.dirty-checkout-pending.v1";
const INSTANCE_FILE: &str = ".agent-runtime-checkout-instance";
const MAX_GIT_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 1024 * 1024;
const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 32 * 1024 * 1024;
const MAX_STATE_FILE_BYTES: usize = 16 * 1024;
const MAX_PENDING_STATE_FILE_BYTES: usize = MAX_STATE_FILE_BYTES * 4 + 4 * 1024;
const MAX_REVOCATION_TOMBSTONES: usize = 64;
const MAX_RECOVERY_DIRECTORY_ENTRIES: usize = 1_024;
const MAX_RECOVERY_STATE_ENTRIES: usize = 256;
const MAX_RECOVERY_AGGREGATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECOVERY_NAME_BYTES: usize = 256 * 1024;
const MAX_REASON_BYTES: usize = 2_000;
const MAX_ENTRY_COUNT: usize = 100_000;
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(50);
const DEFAULT_LEASE_TTL_SECONDS: u64 = 8 * 60 * 60;
const MAX_LEASE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
const UNBORN_HEAD_PREFIX: &str = "unborn:";
const TRUSTED_GIT_PATHS: &[&str] = &["/usr/bin/git", "/bin/git"];
#[cfg(target_os = "linux")]
const LINUX_UID_MAP_PATH: &str = "/proc/self/uid_map";
#[cfg(target_os = "linux")]
const LINUX_OVERFLOW_UID_PATH: &str = "/proc/sys/kernel/overflowuid";
#[cfg(target_os = "linux")]
const MAX_LINUX_UID_MAP_BYTES: usize = 4 * 1024;
const SNAPSHOT_WORKER_ENV: &str = "NILS_GIT_CLI_INTERNAL_DIRTY_SNAPSHOT_WORKER";
const SNAPSHOT_WORKER_CHECKOUT_ENV: &str = "NILS_GIT_CLI_INTERNAL_DIRTY_SNAPSHOT_CHECKOUT";
const PROCESS_SUPERVISOR_ENV: &str = "NILS_GIT_CLI_INTERNAL_PROCESS_SUPERVISOR";
const PROCESS_SUPERVISOR_CAPABILITY_FD_ENV: &str =
    "NILS_GIT_CLI_INTERNAL_PROCESS_SUPERVISOR_CAPABILITY_FD";
const PROCESS_SUPERVISOR_CAPABILITY: &[u8] = b"nils-git-cli-process-supervisor-v1";
const PROCESS_SUPERVISOR_COMPLETION: &[u8] = b"nils-git-cli-process-supervisor-completion-v1";
const PROCESS_SUPERVISOR_COMPLETION_BYTES: usize = PROCESS_SUPERVISOR_COMPLETION.len() + 6;
const SNAPSHOT_WORKER_SCHEMA: &str = "nils-git-cli.dirty-snapshot-worker.v1";
const CHALLENGE_TOKEN_BYTES: usize = 64;
const CHALLENGE_DESCRIPTOR_TIMEOUT: Duration = Duration::from_secs(5);

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyCheckoutErrorKind {
    CleanCheckout,
    UnsupportedGitState,
    ChallengeExpired,
    ChallengeReused,
    ChallengeDrift,
    ForeignLease,
    MalformedState,
    InvalidInput,
    Timeout,
    ResourceUnavailable,
    SnapshotFailed,
    AdoptionFailed,
    RevocationFailed,
}

impl DirtyCheckoutErrorKind {
    pub const fn code(self) -> &'static str {
        match self {
            Self::CleanCheckout => "dirty-checkout-clean",
            Self::UnsupportedGitState => "dirty-checkout-unsupported-git-state",
            Self::ChallengeExpired => "dirty-checkout-challenge-expired",
            Self::ChallengeReused => "dirty-checkout-challenge-reused",
            Self::ChallengeDrift => "dirty-checkout-challenge-drift",
            Self::ForeignLease => "dirty-checkout-foreign-lease",
            Self::MalformedState => "dirty-checkout-malformed-state",
            Self::InvalidInput => "dirty-checkout-invalid-input",
            Self::Timeout => "dirty-checkout-timeout",
            Self::ResourceUnavailable => "dirty-checkout-resource-unavailable",
            Self::SnapshotFailed => "dirty-checkout-snapshot-failed",
            Self::AdoptionFailed => "dirty-checkout-adoption-failed",
            Self::RevocationFailed => "dirty-checkout-revoke-failed",
        }
    }

    const fn exit_code(self) -> i32 {
        match self {
            Self::Timeout
            | Self::ResourceUnavailable
            | Self::SnapshotFailed
            | Self::AdoptionFailed
            | Self::RevocationFailed => exit::RUNTIME,
            Self::CleanCheckout
            | Self::UnsupportedGitState
            | Self::ChallengeExpired
            | Self::ChallengeReused
            | Self::ChallengeDrift
            | Self::ForeignLease
            | Self::MalformedState
            | Self::InvalidInput => exit::DATA,
        }
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct DirtyCheckoutError {
    kind: DirtyCheckoutErrorKind,
    message: Box<str>,
}

impl DirtyCheckoutError {
    fn new(kind: DirtyCheckoutErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into().into_boxed_str(),
        }
    }

    pub const fn kind(&self) -> DirtyCheckoutErrorKind {
        self.kind
    }

    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }
}

fn domain_error(kind: DirtyCheckoutErrorKind, message: impl Into<String>) -> anyhow::Error {
    DirtyCheckoutError::new(kind, message).into()
}

fn command_error(
    error: anyhow::Error,
    fallback: DirtyCheckoutErrorKind,
    message: impl AsRef<str>,
) -> anyhow::Error {
    if error.downcast_ref::<DirtyCheckoutError>().is_some() {
        error
    } else {
        domain_error(fallback, format!("{}: {error}", message.as_ref()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirtySnapshot {
    pub schema: &'static str,
    pub repository_key: String,
    pub checkout_key: String,
    pub checkout_instance: String,
    pub snapshot_id: String,
    pub head_oid: String,
    pub branch_ref_digest: String,
    pub tracked_entries: usize,
    pub untracked_entries: usize,
    pub hashed_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptionReceipt {
    pub receipt_id: String,
    pub snapshot_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWorkerSnapshot {
    schema: String,
    repository_key: String,
    checkout_key: String,
    checkout_instance: String,
    snapshot_id: String,
    head_oid: String,
    branch_ref_digest: String,
    tracked_entries: usize,
    untracked_entries: usize,
    hashed_bytes: u64,
}

impl From<DirtySnapshot> for SnapshotWorkerSnapshot {
    fn from(snapshot: DirtySnapshot) -> Self {
        Self {
            schema: snapshot.schema.to_string(),
            repository_key: snapshot.repository_key,
            checkout_key: snapshot.checkout_key,
            checkout_instance: snapshot.checkout_instance,
            snapshot_id: snapshot.snapshot_id,
            head_oid: snapshot.head_oid,
            branch_ref_digest: snapshot.branch_ref_digest,
            tracked_entries: snapshot.tracked_entries,
            untracked_entries: snapshot.untracked_entries,
            hashed_bytes: snapshot.hashed_bytes,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWorkerError {
    code: String,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWorkerResponse {
    schema: String,
    snapshot: Option<SnapshotWorkerSnapshot>,
    error: Option<SnapshotWorkerError>,
}

#[derive(Debug, PartialEq, Eq)]
struct CheckoutIdentity {
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
    repository_key: String,
    checkout_key: String,
    checkout_instance: String,
}

#[derive(Debug, Clone)]
struct IndexEntry {
    mode: Vec<u8>,
    oid: Vec<u8>,
    path: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeRecord {
    schema: String,
    token_digest: String,
    session_key: String,
    repository_key: String,
    checkout_key: String,
    checkout_instance: String,
    snapshot_id: String,
    head_oid: String,
    branch_ref_digest: String,
    authorization_turn_digest: String,
    issued_at: u64,
    expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdoptionRecord {
    schema: String,
    receipt_schema: String,
    receipt_id: String,
    snapshot_id: String,
    authorization_turn_digest: String,
    reason_digest: String,
    adopted_at: u64,
    challenge_issued_at: u64,
    challenge_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum LeaseRecord {
    V1(LeaseV1Wire),
    V2(Box<LeaseV2Wire>),
}

impl LeaseRecord {
    fn schema(&self) -> &str {
        match self {
            Self::V1(lease) => &lease.schema,
            Self::V2(lease) => &lease.schema,
        }
    }

    fn session_key(&self) -> &str {
        match self {
            Self::V1(lease) => &lease.session_key,
            Self::V2(lease) => &lease.session_key,
        }
    }

    fn checkout_instance(&self) -> &str {
        match self {
            Self::V1(lease) => &lease.checkout_instance,
            Self::V2(lease) => &lease.checkout_instance,
        }
    }

    fn acquired_at(&self) -> u64 {
        match self {
            Self::V1(lease) => lease.acquired_at,
            Self::V2(lease) => lease.acquired_at,
        }
    }

    fn refreshed_at(&self) -> u64 {
        match self {
            Self::V1(lease) => lease.refreshed_at,
            Self::V2(lease) => lease.refreshed_at,
        }
    }

    fn expires_at(&self) -> u64 {
        match self {
            Self::V1(lease) => lease.expires_at,
            Self::V2(lease) => lease.expires_at,
        }
    }

    fn adoption(&self) -> Option<&AdoptionRecord> {
        match self {
            Self::V1(_) => None,
            Self::V2(lease) => Some(&lease.adoption),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseV1Wire {
    schema: String,
    session_key: String,
    checkout_instance: String,
    checkout_root: String,
    checkout_git_dir: String,
    acquired_at: u64,
    refreshed_at: u64,
    expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseV2Wire {
    schema: String,
    session_key: String,
    checkout_instance: String,
    checkout_root: String,
    checkout_git_dir: String,
    checkout_root_bytes: String,
    checkout_git_dir_bytes: String,
    acquired_at: u64,
    refreshed_at: u64,
    expires_at: u64,
    adoption: AdoptionRecord,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LeaseWire {
    V1(LeaseV1Wire),
    V2(Box<LeaseV2Wire>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptRecord {
    schema: String,
    receipt_id: String,
    session_key: String,
    repository_key: String,
    checkout_key: String,
    checkout_instance: String,
    snapshot_id: String,
    authorization_turn_digest: String,
    reason_digest: String,
    challenge_digest: String,
    adopted_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingAdoptionRecord {
    schema: String,
    receipt_id: String,
    token_digest: String,
    challenge_digest: String,
    session_key: String,
    checkout_instance: String,
    snapshot_id: String,
    predecessor_receipt_id: Option<String>,
    predecessor_receipt_digest: Option<String>,
    predecessor_spent_challenge_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    predecessor_lease_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    predecessor_lease_bytes: Option<Vec<u8>>,
}

#[derive(Debug)]
struct SnapshotPass {
    snapshot: DirtySnapshot,
}

#[derive(Debug, Default)]
struct SnapshotBudget {
    capture_bytes: usize,
    path_bytes: usize,
    file_bytes: u64,
    traversal_entries: usize,
}

impl SnapshotBudget {
    fn output_limit(&self, command_limit: usize) -> Result<usize> {
        let remaining = MAX_CAPTURE_BYTES.saturating_sub(self.capture_bytes);
        if remaining == 0 {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot aggregate capture budget is exhausted",
            ));
        }
        Ok(command_limit.min(remaining))
    }

    fn charge_output(&mut self, output: &std::process::Output) -> Result<()> {
        let captured = output.stdout.len().saturating_add(output.stderr.len());
        self.capture_bytes = self.capture_bytes.checked_add(captured).ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot aggregate capture budget overflowed",
            )
        })?;
        if self.capture_bytes > MAX_CAPTURE_BYTES {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot aggregate capture budget is exceeded",
            ));
        }
        Ok(())
    }

    fn charge_path(&mut self, bytes: usize) -> Result<()> {
        self.path_bytes = self.path_bytes.checked_add(bytes).ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot path-byte budget overflowed",
            )
        })?;
        if self.path_bytes > MAX_PATH_BYTES {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot path-byte budget is exceeded",
            ));
        }
        Ok(())
    }

    fn charge_traversal_entry(&mut self) -> Result<()> {
        self.traversal_entries = self.traversal_entries.checked_add(1).ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "checkout filesystem entry count overflowed",
            )
        })?;
        if self.traversal_entries > MAX_ENTRY_COUNT {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "checkout filesystem entry count exceeds the supported limit",
            ));
        }
        Ok(())
    }
}

struct FramedHasher(Sha256);

impl FramedHasher {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn field(&mut self, tag: &[u8], value: &[u8]) {
        self.0.update((tag.len() as u32).to_be_bytes());
        self.0.update(tag);
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn finish(self) -> String {
        hex_bytes(&self.0.finalize())
    }
}

pub fn dirty_snapshot(checkout: &Path) -> Result<DirtySnapshot> {
    dirty_snapshot_worker(checkout, deadline_after(SNAPSHOT_TIMEOUT)?)
}

fn dirty_snapshot_cli(checkout: &Path) -> Result<DirtySnapshot> {
    dirty_snapshot_cli_until(checkout, deadline_after(SNAPSHOT_TIMEOUT)?)
}

fn dirty_snapshot_cli_until(checkout: &Path, deadline: Instant) -> Result<DirtySnapshot> {
    let executable = snapshot_worker_executable()?;
    executable.revalidate()?;
    ensure_deadline(deadline)?;
    let mut command = Command::new(executable.command_path());
    command
        .env(SNAPSHOT_WORKER_ENV, "1")
        .env(SNAPSHOT_WORKER_CHECKOUT_ENV, checkout.as_os_str())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let output = run_snapshot_worker_command(&mut command, deadline)?;
    decode_snapshot_worker_output(output)
}

fn decode_snapshot_worker_output(output: std::process::Output) -> Result<DirtySnapshot> {
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::SnapshotFailed,
            "dirty snapshot worker failed without a valid response",
        ));
    }
    let response: SnapshotWorkerResponse =
        serde_json::from_slice(&output.stdout).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::SnapshotFailed,
                "dirty snapshot worker response is malformed",
            )
        })?;
    if response.schema != SNAPSHOT_WORKER_SCHEMA {
        return Err(domain_error(
            DirtyCheckoutErrorKind::SnapshotFailed,
            "dirty snapshot worker response schema is unsupported",
        ));
    }
    match (response.snapshot, response.error) {
        (Some(snapshot), None) if valid_worker_snapshot(&snapshot) => Ok(DirtySnapshot {
            schema: SNAPSHOT_SCHEMA,
            repository_key: snapshot.repository_key,
            checkout_key: snapshot.checkout_key,
            checkout_instance: snapshot.checkout_instance,
            snapshot_id: snapshot.snapshot_id,
            head_oid: snapshot.head_oid,
            branch_ref_digest: snapshot.branch_ref_digest,
            tracked_entries: snapshot.tracked_entries,
            untracked_entries: snapshot.untracked_entries,
            hashed_bytes: snapshot.hashed_bytes,
        }),
        (Some(_), None) => Err(domain_error(
            DirtyCheckoutErrorKind::SnapshotFailed,
            "dirty snapshot worker success response is semantically invalid",
        )),
        (None, Some(error)) => {
            let kind = error_kind_from_code(&error.code).ok_or_else(|| {
                domain_error(
                    DirtyCheckoutErrorKind::SnapshotFailed,
                    "dirty snapshot worker returned an unsupported error code",
                )
            })?;
            Err(domain_error(kind, error.message))
        }
        _ => Err(domain_error(
            DirtyCheckoutErrorKind::SnapshotFailed,
            "dirty snapshot worker response is inconsistent",
        )),
    }
}

fn valid_worker_snapshot(snapshot: &SnapshotWorkerSnapshot) -> bool {
    snapshot.schema == SNAPSHOT_SCHEMA
        && is_lower_hex(&snapshot.repository_key, 64)
        && is_lower_hex(&snapshot.checkout_key, 64)
        && is_lower_hex(&snapshot.checkout_instance, 32)
        && is_lower_hex(&snapshot.snapshot_id, 64)
        && valid_head_identity(&snapshot.head_oid)
        && is_lower_hex(&snapshot.branch_ref_digest, 64)
        && snapshot
            .tracked_entries
            .checked_add(snapshot.untracked_entries)
            .is_some_and(|entries| entries <= MAX_ENTRY_COUNT)
        && snapshot.hashed_bytes <= MAX_TOTAL_BYTES
}

pub(crate) fn run_internal_snapshot_worker() -> Option<i32> {
    if env::var_os(SNAPSHOT_WORKER_ENV).as_deref() != Some(OsStr::new("1")) {
        return None;
    }
    let result = env::var_os(SNAPSHOT_WORKER_CHECKOUT_ENV)
        .map(PathBuf::from)
        .context("dirty snapshot worker checkout is unavailable")
        .and_then(|checkout| {
            let deadline = Instant::now() + SNAPSHOT_TIMEOUT;
            dirty_snapshot_worker(&checkout, deadline)
        });
    let response = match result {
        Ok(snapshot) => SnapshotWorkerResponse {
            schema: SNAPSHOT_WORKER_SCHEMA.to_string(),
            snapshot: Some(snapshot.into()),
            error: None,
        },
        Err(error) => {
            let domain = error.downcast_ref::<DirtyCheckoutError>();
            SnapshotWorkerResponse {
                schema: SNAPSHOT_WORKER_SCHEMA.to_string(),
                snapshot: None,
                error: Some(SnapshotWorkerError {
                    code: domain
                        .map_or(DirtyCheckoutErrorKind::SnapshotFailed.code(), |error| {
                            error.code()
                        })
                        .to_string(),
                    message: error.to_string(),
                }),
            }
        }
    };
    match serde_json::to_writer(io::stdout().lock(), &response) {
        Ok(()) => Some(0),
        Err(_) => Some(exit::RUNTIME),
    }
}

const PROCESS_SUPERVISOR_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessSupervisorCompletionKind {
    Target = 1,
    Terminated = 2,
    InternalFailure = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessSupervisorCompletion {
    kind: ProcessSupervisorCompletionKind,
    cleanup_complete: bool,
    exit_code: i32,
}

fn write_process_supervisor_completion<W: Write>(
    writer: &mut W,
    completion: ProcessSupervisorCompletion,
) -> io::Result<()> {
    let mut frame = Vec::with_capacity(PROCESS_SUPERVISOR_COMPLETION_BYTES);
    frame.extend_from_slice(PROCESS_SUPERVISOR_COMPLETION);
    frame.push(completion.kind as u8);
    frame.push(u8::from(completion.cleanup_complete));
    frame.extend_from_slice(&completion.exit_code.to_be_bytes());
    writer.write_all(&frame)
}

fn read_process_supervisor_completion<R: Read>(
    reader: &mut R,
) -> Result<ProcessSupervisorCompletion> {
    let mut frame = vec![0_u8; PROCESS_SUPERVISOR_COMPLETION_BYTES];
    reader
        .read_exact(&mut frame)
        .context("process supervisor completion is unavailable")?;
    let mut trailing = [0_u8; 1];
    if reader
        .read(&mut trailing)
        .context("process supervisor completion could not be sealed")?
        != 0
        || &frame[..PROCESS_SUPERVISOR_COMPLETION.len()] != PROCESS_SUPERVISOR_COMPLETION
    {
        return Err(process_scan_resource_error());
    }
    let kind_offset = PROCESS_SUPERVISOR_COMPLETION.len();
    let kind = match frame[kind_offset] {
        1 => ProcessSupervisorCompletionKind::Target,
        2 => ProcessSupervisorCompletionKind::Terminated,
        3 => ProcessSupervisorCompletionKind::InternalFailure,
        _ => return Err(process_scan_resource_error()),
    };
    let cleanup_complete = match frame[kind_offset + 1] {
        0 => false,
        1 => true,
        _ => return Err(process_scan_resource_error()),
    };
    let exit_code = i32::from_be_bytes(
        frame[kind_offset + 2..]
            .try_into()
            .map_err(|_| process_scan_resource_error())?,
    );
    Ok(ProcessSupervisorCompletion {
        kind,
        cleanup_complete,
        exit_code,
    })
}

static PROCESS_SUPERVISOR_TERMINATE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

extern "C" fn request_process_supervisor_termination(_signal: libc::c_int) {
    PROCESS_SUPERVISOR_TERMINATE.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn supervisor_parent_is_trusted() -> bool {
    let Ok(current) = env::current_exe() else {
        return false;
    };
    let Ok(current_metadata) = fs::metadata(current) else {
        return false;
    };
    let parent = unsafe { libc::getppid() };

    #[cfg(target_os = "linux")]
    let parent_metadata = fs::metadata(format!("/proc/{parent}/exe"));

    #[cfg(target_os = "macos")]
    let parent_metadata = {
        let mut bytes = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let length =
            unsafe { libc::proc_pidpath(parent, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
        if length <= 0 {
            return false;
        }
        bytes.truncate(length as usize);
        fs::metadata(PathBuf::from(OsString::from_vec(bytes)))
    };

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let parent_metadata: io::Result<Metadata> = Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process parent identity is unsupported",
    ));

    parent_metadata.is_ok_and(|metadata| same_metadata(&current_metadata, &metadata))
}

#[cfg(target_os = "linux")]
fn linux_peer_credentials_match(
    credentials: libc::ucred,
    parent: libc::pid_t,
    effective_uid: libc::uid_t,
) -> bool {
    credentials.pid == parent && credentials.uid == effective_uid
}

#[derive(Debug)]
struct AuthenticatedProcessSupervisor {
    channel: std::os::unix::net::UnixStream,
    deadline: Instant,
}

fn monotonic_clock_nanos() -> Result<u64> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut value) } != 0 {
        return Err(process_scan_resource_error());
    }
    u64::try_from(value.tv_sec)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000_000))
        .and_then(|nanos| {
            u64::try_from(value.tv_nsec)
                .ok()
                .and_then(|fraction| nanos.checked_add(fraction))
        })
        .ok_or_else(process_scan_resource_error)
}

fn monotonic_deadline_nanos(deadline: Instant) -> Result<u64> {
    let now_nanos = monotonic_clock_nanos()?;
    let now = Instant::now();
    now_nanos
        .checked_add(
            u64::try_from(deadline.saturating_duration_since(now).as_nanos())
                .map_err(|_| process_scan_resource_error())?,
        )
        .ok_or_else(process_scan_resource_error)
}

fn instant_from_monotonic_deadline(deadline_nanos: u64) -> Result<Instant> {
    let now_nanos = monotonic_clock_nanos()?;
    let remaining = Duration::from_nanos(deadline_nanos.saturating_sub(now_nanos));
    if remaining > SNAPSHOT_TIMEOUT {
        return Err(process_scan_resource_error());
    }
    Instant::now()
        .checked_add(remaining)
        .ok_or_else(process_scan_resource_error)
}

fn process_supervisor_owner_is_lost(channel: &std::os::unix::net::UnixStream) -> Result<bool> {
    let mut byte = [0_u8; 1];
    let result = unsafe {
        libc::recv(
            channel.as_raw_fd(),
            byte.as_mut_ptr().cast(),
            byte.len(),
            libc::MSG_DONTWAIT | libc::MSG_PEEK,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    if result > 0 {
        return Err(process_scan_resource_error());
    }
    let error = io::Error::last_os_error();
    if matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    ) {
        Ok(false)
    } else {
        Err(process_scan_resource_error())
    }
}

fn validate_process_supervisor_capability() -> Result<AuthenticatedProcessSupervisor> {
    validate_process_supervisor_capability_with(supervisor_parent_is_trusted)
}

fn validate_process_supervisor_capability_with<F>(
    parent_is_trusted: F,
) -> Result<AuthenticatedProcessSupervisor>
where
    F: FnOnce() -> bool,
{
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixStream;

    if !parent_is_trusted() {
        return Err(process_scan_resource_error());
    }
    let descriptor = env::var_os(PROCESS_SUPERVISOR_CAPABILITY_FD_ENV)
        .and_then(|value| {
            value
                .to_str()
                .and_then(|value| value.parse::<libc::c_int>().ok())
        })
        .filter(|descriptor| *descriptor == libc::STDIN_FILENO || *descriptor > libc::STDERR_FILENO)
        .ok_or_else(process_scan_resource_error)?;
    let descriptor = if descriptor == libc::STDIN_FILENO {
        let duplicated = unsafe {
            libc::fcntl(
                libc::STDIN_FILENO,
                libc::F_DUPFD_CLOEXEC,
                libc::STDERR_FILENO + 1,
            )
        };
        if duplicated < 0 {
            return Err(process_scan_resource_error());
        }
        if !restore_standard_input() {
            let _ = unsafe { libc::close(duplicated) };
            return Err(process_scan_resource_error());
        }
        duplicated
    } else {
        descriptor
    };

    #[cfg(target_os = "linux")]
    {
        let mut credentials = std::mem::MaybeUninit::<libc::ucred>::zeroed();
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                descriptor,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                credentials.as_mut_ptr().cast(),
                &mut length,
            )
        };
        if result != 0 || length as usize != std::mem::size_of::<libc::ucred>() {
            return Err(process_scan_resource_error());
        }
        let credentials = unsafe { credentials.assume_init() };
        if !linux_peer_credentials_match(credentials, unsafe { libc::getppid() }, unsafe {
            libc::geteuid()
        }) {
            return Err(process_scan_resource_error());
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut uid = 0;
        let mut gid = 0;
        if unsafe { libc::getpeereid(descriptor, &mut uid, &mut gid) } != 0
            || uid != unsafe { libc::geteuid() }
        {
            return Err(process_scan_resource_error());
        }
    }

    let mut request = vec![0_u8; PROCESS_SUPERVISOR_CAPABILITY.len() + 8];
    let mut stream = unsafe { UnixStream::from_raw_fd(descriptor) };
    stream
        .read_exact(&mut request)
        .context("process supervisor capability is unavailable")?;
    if &request[..PROCESS_SUPERVISOR_CAPABILITY.len()] != PROCESS_SUPERVISOR_CAPABILITY {
        return Err(process_scan_resource_error());
    }
    let deadline_nanos = u64::from_be_bytes(
        request[PROCESS_SUPERVISOR_CAPABILITY.len()..]
            .try_into()
            .map_err(|_| process_scan_resource_error())?,
    );
    let deadline = instant_from_monotonic_deadline(deadline_nanos)?;
    let descriptor_flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if descriptor_flags < 0
        || unsafe {
            libc::fcntl(
                descriptor,
                libc::F_SETFD,
                descriptor_flags | libc::FD_CLOEXEC,
            )
        } < 0
    {
        return Err(process_scan_resource_error());
    }
    Ok(AuthenticatedProcessSupervisor {
        channel: stream,
        deadline,
    })
}

pub(crate) fn run_internal_process_supervisor() -> Option<i32> {
    if env::var_os(PROCESS_SUPERVISOR_ENV).as_deref() != Some(OsStr::new("1")) {
        return None;
    }
    let authenticated = match validate_process_supervisor_capability() {
        Ok(authenticated) => authenticated,
        Err(_) => return Some(exit::RUNTIME),
    };
    let mut arguments = env::args_os().skip(1);
    let Some(program) = arguments.next() else {
        return Some(exit::RUNTIME);
    };
    let arguments: Vec<OsString> = arguments.collect();
    Some(
        supervise_process_with_cleanup_proof(
            program,
            &arguments,
            Some(authenticated.channel),
            authenticated.deadline,
        )
        .unwrap_or(exit::RUNTIME),
    )
}

fn with_process_supervisor_cleanup_proof<F>(
    mut completion_channel: Option<std::os::unix::net::UnixStream>,
    run: F,
) -> Result<i32>
where
    F: FnOnce(Option<&mut std::os::unix::net::UnixStream>) -> Result<i32>,
{
    PROCESS_SUPERVISOR_TERMINATE.store(false, std::sync::atomic::Ordering::SeqCst);
    let previous = unsafe {
        libc::signal(
            libc::SIGTERM,
            request_process_supervisor_termination as *const () as libc::sighandler_t,
        )
    };
    if previous == libc::SIG_ERR {
        return Err(process_scan_resource_error());
    }
    let result = run(completion_channel.as_mut());
    if result.is_err()
        && let Some(channel) = &mut completion_channel
    {
        let _ = write_process_supervisor_completion(
            channel,
            ProcessSupervisorCompletion {
                kind: ProcessSupervisorCompletionKind::InternalFailure,
                cleanup_complete: false,
                exit_code: exit::RUNTIME,
            },
        );
    }
    result
}

fn supervise_process_with_cleanup_proof(
    program: OsString,
    arguments: &[OsString],
    completion_channel: Option<std::os::unix::net::UnixStream>,
    deadline: Instant,
) -> Result<i32> {
    with_process_supervisor_cleanup_proof(completion_channel, |completion_channel| {
        supervise_process_with(program, arguments, completion_channel, deadline, || {
            PROCESS_SUPERVISOR_TERMINATE.load(std::sync::atomic::Ordering::SeqCst)
        })
    })
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn supervise_process_with_cleanup_proof_and_owner<T>(
    program: OsString,
    arguments: &[OsString],
    completion_channel: Option<std::os::unix::net::UnixStream>,
    deadline: Instant,
    owner: &mut T,
) -> Result<i32>
where
    T: OwnedProcessTracker,
{
    with_process_supervisor_cleanup_proof(completion_channel, |completion_channel| {
        supervise_process_with_owner(
            program,
            arguments,
            owner,
            completion_channel,
            deadline,
            || PROCESS_SUPERVISOR_TERMINATE.load(std::sync::atomic::Ordering::SeqCst),
        )
    })
}

fn supervise_process_with<F>(
    program: OsString,
    arguments: &[OsString],
    completion_channel: Option<&mut std::os::unix::net::UnixStream>,
    deadline: Instant,
    should_terminate: F,
) -> Result<i32>
where
    F: FnMut() -> bool,
{
    let mut owner = ProcessOwner::new()?;
    supervise_process_with_owner(
        program,
        arguments,
        &mut owner,
        completion_channel,
        deadline,
        should_terminate,
    )
}

fn supervise_process_with_owner<T, F>(
    program: OsString,
    arguments: &[OsString],
    owner: &mut T,
    mut completion_channel: Option<&mut std::os::unix::net::UnixStream>,
    deadline: Instant,
    mut should_terminate: F,
) -> Result<i32>
where
    T: OwnedProcessTracker,
    F: FnMut() -> bool,
{
    let mut command = Command::new(program);
    command
        .args(arguments)
        .env_remove(PROCESS_SUPERVISOR_ENV)
        .env_remove(PROCESS_SUPERVISOR_CAPABILITY_FD_ENV)
        .stdin(Stdio::null());
    let mut child = command
        .spawn()
        .context("supervised process could not be started")?;
    loop {
        let owner_lost = match completion_channel.as_deref() {
            Some(channel) => process_supervisor_owner_is_lost(channel)?,
            None => false,
        };
        let deadline_expired = Instant::now() >= deadline;
        let signal_requested = should_terminate();
        if owner_lost || deadline_expired || signal_requested {
            terminate_owned_processes(owner, &mut child)?;
            if !owner_lost && let Some(channel) = &mut completion_channel {
                write_process_supervisor_completion(
                    channel,
                    ProcessSupervisorCompletion {
                        kind: ProcessSupervisorCompletionKind::Terminated,
                        cleanup_complete: true,
                        exit_code: exit::RUNTIME,
                    },
                )
                .context("process supervisor completion could not be delivered")?;
            }
            return Ok(exit::RUNTIME);
        }
        let refresh_deadline = deadline.min(
            Instant::now()
                .checked_add(PROCESS_CLEANUP_TIMEOUT)
                .unwrap_or(deadline),
        );
        if let Err(error) = owner.refresh(refresh_deadline) {
            let deadline_expired = Instant::now() >= deadline;
            return match terminate_owned_processes(owner, &mut child) {
                Ok(()) if deadline_expired => {
                    if let Some(channel) = &mut completion_channel {
                        write_process_supervisor_completion(
                            channel,
                            ProcessSupervisorCompletion {
                                kind: ProcessSupervisorCompletionKind::Terminated,
                                cleanup_complete: true,
                                exit_code: exit::RUNTIME,
                            },
                        )
                        .context("process supervisor completion could not be delivered")?;
                    }
                    Ok(exit::RUNTIME)
                }
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(cleanup_error),
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_owned_processes(owner, &mut child)?;
                let Some(exit_code) = status.code() else {
                    return Err(domain_error(
                        DirtyCheckoutErrorKind::ResourceUnavailable,
                        "supervised process terminated without an exit status",
                    ));
                };
                if let Some(channel) = &mut completion_channel {
                    write_process_supervisor_completion(
                        channel,
                        ProcessSupervisorCompletion {
                            kind: ProcessSupervisorCompletionKind::Target,
                            cleanup_complete: true,
                            exit_code,
                        },
                    )
                    .context("process supervisor completion could not be delivered")?;
                }
                return Ok(exit_code);
            }
            Ok(None) => std::thread::sleep(PROCESS_SUPERVISOR_POLL_INTERVAL),
            Err(error) => {
                return match terminate_owned_processes(owner, &mut child) {
                    Ok(()) => Err(error).context("supervised process status failed"),
                    Err(cleanup_error) => Err(cleanup_error),
                };
            }
        }
    }
}

struct SnapshotWorkerExecutable {
    source_path: PathBuf,
    command_path: PathBuf,
    file: File,
    metadata: Metadata,
    digest: [u8; 32],
}

struct WorkerDigestCache {
    file: File,
    metadata: Metadata,
    digest: [u8; 32],
}

static WORKER_DIGEST_CACHE: OnceLock<Mutex<Option<WorkerDigestCache>>> = OnceLock::new();

impl SnapshotWorkerExecutable {
    fn command_path(&self) -> &Path {
        &self.command_path
    }

    fn revalidate(&self) -> Result<()> {
        let file_metadata = self
            .file
            .metadata()
            .context("dirty snapshot worker executable descriptor is unavailable")?;
        validate_worker_file_metadata(&file_metadata)?;
        if !same_metadata(&self.metadata, &file_metadata)
            && (!same_worker_descriptor_metadata(&self.metadata, &file_metadata)
                || worker_file_digest(&self.file, file_metadata.len())? != self.digest)
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot worker executable changed after validation",
            ));
        }
        if self.command_path == self.source_path {
            validate_worker_path_permissions(&self.source_path)?;
            let path_metadata = fs::metadata(&self.source_path)
                .context("dirty snapshot worker executable could not be revalidated")?;
            if !same_metadata(&self.metadata, &path_metadata) {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::ResourceUnavailable,
                    "dirty snapshot worker executable changed after validation",
                ));
            }
        }
        Ok(())
    }
}

fn same_worker_descriptor_metadata(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.mode() == right.mode()
        && left.uid() == right.uid()
        && left.gid() == right.gid()
        && left.nlink() == right.nlink()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
}

fn worker_file_digest(file: &File, length: u64) -> Result<[u8; 32]> {
    ensure!(
        length <= MAX_CAPTURE_BYTES as u64,
        "dirty snapshot worker executable exceeds the supported size limit"
    );
    let mut digest = Sha256::new();
    let mut offset = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while offset < length {
        let read_limit = usize::try_from((length - offset).min(buffer.len() as u64))
            .expect("bounded worker executable read size");
        let read = file
            .read_at(&mut buffer[..read_limit], offset)
            .context("dirty snapshot worker executable could not be hashed")?;
        ensure!(
            read > 0,
            "dirty snapshot worker executable changed while hashing"
        );
        digest.update(&buffer[..read]);
        offset += read as u64;
    }
    let mut trailing = [0_u8; 1];
    ensure!(
        file.read_at(&mut trailing, length)
            .context("dirty snapshot worker executable final read failed")?
            == 0,
        "dirty snapshot worker executable changed while hashing"
    );
    Ok(digest.finalize().into())
}

fn cached_worker_file_digest(file: &File, metadata: &Metadata) -> Result<[u8; 32]> {
    let cache = WORKER_DIGEST_CACHE.get_or_init(|| Mutex::new(None));
    let mut cache = cache.lock().map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty snapshot worker executable digest cache is unavailable",
        )
    })?;
    if let Some(cached) = cache.as_ref() {
        let cached_metadata = cached
            .file
            .metadata()
            .context("cached dirty snapshot worker executable is unavailable")?;
        if same_metadata(&cached.metadata, &cached_metadata)
            && same_metadata(&cached_metadata, metadata)
        {
            return Ok(cached.digest);
        }
    }

    let digest = worker_file_digest(file, metadata.len())?;
    let hashed_metadata = file
        .metadata()
        .context("dirty snapshot worker executable could not be revalidated after hashing")?;
    if !same_metadata(metadata, &hashed_metadata) {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty snapshot worker executable changed during validation",
        ));
    }
    *cache = Some(WorkerDigestCache {
        file: file
            .try_clone()
            .context("dirty snapshot worker executable descriptor could not be retained")?,
        metadata: hashed_metadata,
        digest,
    });
    Ok(digest)
}

#[cfg(test)]
static PRIVATE_TEST_WORKER: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// Age past which an orphaned staging copy is assumed to belong to a crashed
/// install rather than a live one — installs complete in well under a second.
#[cfg(test)]
const PRIVATE_TEST_WORKER_STAGING_TTL: Duration = Duration::from_secs(600);

/// Provide a hardened private copy of the worker binary for tests.
///
/// Under the shared-dev umask the on-disk `git-cli` binary is group-writable,
/// so production's descriptor validation rejects it and tests need a hardened
/// (`0o500`) copy. This helper historically minted a fresh `tempfile::TempDir`
/// per call; when a test process was killed or aborted before `Drop` ran
/// (nextest timeouts, `panic = "abort"`, coverage) the ~17 MB copy leaked into
/// `/tmp`, and tens of thousands accumulated on one host. Install the copy once
/// at a stable, per-source path and reuse it across calls, processes, and runs
/// so at most one copy per source binary ever exists on disk. See issue #1323.
///
/// Trust is fail-closed: a hostile pre-created per-user directory (foreign
/// owner, group/other writable, or a symlink) makes this error out rather than
/// adopt untrusted state. Adopt-first keeps concurrent reinstalls bounded to
/// cold start; the resulting shared-path rewrite window can only be observed by
/// the non-Linux (`command_path == candidate`) re-stat in
/// [`snapshot_worker_executable`]. Linux execs via `/proc/self/fd`; non-Linux
/// launch still validates the canonical copy path and its ancestry.
#[cfg(test)]
fn private_test_worker_executable(candidate: PathBuf) -> Result<PathBuf> {
    let source = File::open(&candidate)
        .context("dirty snapshot worker test executable could not be opened")?;
    let source_metadata = source
        .metadata()
        .context("dirty snapshot worker test executable metadata is unavailable")?;
    if source_metadata.mode() & 0o022 == 0 || source_metadata.uid() != unsafe { libc::geteuid() } {
        return Ok(candidate);
    }

    let cache = PRIVATE_TEST_WORKER.get_or_init(|| Mutex::new(None));
    let mut cache = cache.lock().map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "private dirty snapshot worker test cache is unavailable",
        )
    })?;

    let private_dir = private_test_worker_dir()?;
    let private_candidate = private_dir.join(private_test_worker_name(&candidate));
    // Fast path: this process already installed and validated the reusable copy.
    if cache.as_deref() == Some(private_candidate.as_path()) {
        return Ok(private_candidate);
    }

    // Off the fast path (first touch this process, adopt, or reinstall) reclaim
    // staging files orphaned by an earlier crashed install; steady-state reuse
    // stays on the fast path above and skips this.
    prune_stale_private_test_worker_staging(&private_dir);

    let source_len = source_metadata.len();
    let source_digest = worker_file_digest(&source, source_len)?;
    let hashed_metadata = source
        .metadata()
        .context("dirty snapshot worker test executable could not be revalidated after hashing")?;
    if !same_metadata(&source_metadata, &hashed_metadata) {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty snapshot worker test executable changed during private installation",
        ));
    }

    // Adopt an already-installed copy (this run or a prior one) when it still
    // matches the current binary; reinstall only when it is missing or stale so
    // concurrent readers keep observing a stable inode.
    if !private_test_worker_is_current(&private_candidate, source_len, &source_digest)? {
        install_private_test_worker(&private_candidate, &source, source_len, &source_digest)?;
    }
    *cache = Some(private_candidate.clone());
    Ok(private_candidate)
}

#[cfg(test)]
mod test_worker_root;
#[cfg(test)]
use test_worker_root::private_test_worker_dir;

/// Stable, collision-resistant file name derived from the source path so
/// distinct source binaries (the real target binary, test fixtures) never share
/// a copy, while re-runs of the same binary reuse and overwrite one file.
#[cfg(test)]
fn private_test_worker_name(source: &Path) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(source.as_os_str().as_bytes());
    let digest = hasher.finalize();
    let mut name = String::from("git-cli.");
    for byte in &digest[..16] {
        let _ = write!(name, "{byte:02x}");
    }
    name
}

/// Whether a previously installed copy is still a faithful, hardened copy of the
/// current source binary.
#[cfg(test)]
fn private_test_worker_is_current(
    path: &Path,
    source_len: u64,
    source_digest: &[u8; 32],
) -> Result<bool> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(_) => return Ok(false),
    };
    let metadata = file
        .metadata()
        .context("private dirty snapshot worker test copy metadata is unavailable")?;
    if !metadata.file_type().is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o500
        || metadata.len() != source_len
    {
        return Ok(false);
    }
    Ok(&worker_file_digest(&file, source_len)? == source_digest)
}

/// Atomically install a hardened private copy at `destination`, replacing any
/// stale copy. Staging happens in the same directory so the publishing rename is
/// atomic, and the copy is verified against the validated source before being
/// hardened and published.
#[cfg(test)]
fn install_private_test_worker(
    destination: &Path,
    source: &File,
    source_len: u64,
    source_digest: &[u8; 32],
) -> Result<()> {
    let directory = destination
        .parent()
        .context("private dirty snapshot worker test copy has no parent directory")?;
    let mut staged = tempfile::Builder::new()
        .prefix(".git-cli.")
        .suffix(".tmp")
        .tempfile_in(directory)
        .context("private dirty snapshot worker test copy could not be staged")?;
    let mut reader: &File = source;
    io::copy(&mut reader, staged.as_file_mut())
        .context("dirty snapshot worker test executable could not be copied")?;
    staged
        .as_file()
        .sync_all()
        .context("private dirty snapshot worker test copy could not be synced")?;
    let staged_len = staged
        .as_file()
        .metadata()
        .context("private dirty snapshot worker test copy metadata is unavailable")?
        .len();
    if staged_len != source_len
        || &worker_file_digest(staged.as_file(), staged_len)? != source_digest
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty snapshot worker test executable changed during private installation",
        ));
    }
    staged
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o500))
        .context("private dirty snapshot worker test copy could not be hardened")?;
    staged
        .persist(destination)
        .map_err(|error| error.error)
        .context("private dirty snapshot worker test copy could not be installed")?;
    Ok(())
}

/// Remove `.git-cli.*.tmp` staging files left by a crashed install. Called off
/// the reuse fast path (first touch, adopt, or reinstall). Only files older
/// than the staging TTL are removed, so a concurrent in-progress install is
/// never disturbed, and the published `git-cli.<hash>` copies are left intact.
#[cfg(test)]
fn prune_stale_private_test_worker_staging(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(".git-cli.") || !name.ends_with(".tmp") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= PRIVATE_TEST_WORKER_STAGING_TTL);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn snapshot_worker_executable() -> Result<SnapshotWorkerExecutable> {
    let current = env::current_exe()
        .and_then(fs::canonicalize)
        .context("dirty snapshot worker executable could not be resolved")?;
    let candidate = if current.file_name() == Some(OsStr::new("git-cli")) {
        current
    } else {
        let executable_dir = current
            .parent()
            .context("current executable has no parent directory")?;
        let target_dir = if executable_dir.file_name() == Some(OsStr::new("deps")) {
            executable_dir
                .parent()
                .context("test executable directory has no parent")?
        } else {
            executable_dir
        };
        fs::canonicalize(target_dir.join("git-cli"))
            .context("dirty snapshot worker executable is unavailable")?
    };
    #[cfg(test)]
    let candidate = private_test_worker_executable(candidate)?;
    let file = File::open(&candidate)
        .context("dirty snapshot worker executable descriptor could not be opened")?;
    let metadata = file
        .metadata()
        .context("dirty snapshot worker executable metadata is unavailable")?;
    validate_worker_file_metadata(&metadata)?;
    let digest = cached_worker_file_digest(&file, &metadata)?;
    let command_path = if cfg!(target_os = "linux") {
        PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
    } else {
        candidate.clone()
    };
    if command_path == candidate {
        validate_worker_path_permissions(&candidate)?;
        let path_metadata = fs::metadata(&candidate)
            .context("dirty snapshot worker executable path is unavailable")?;
        if !same_metadata(&metadata, &path_metadata) {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot worker executable changed during validation",
            ));
        }
    }
    Ok(SnapshotWorkerExecutable {
        source_path: candidate,
        command_path,
        file,
        metadata,
        digest,
    })
}

fn validate_worker_file_metadata(metadata: &Metadata) -> Result<()> {
    let owner = metadata.uid();
    let mode = metadata.mode();
    if !metadata.file_type().is_file()
        || mode & 0o111 == 0
        || (owner != 0 && owner != unsafe { libc::geteuid() })
        || mode & 0o022 != 0
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty snapshot worker executable is not trusted",
        ));
    }
    Ok(())
}

fn validate_worker_path_permissions(candidate: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(candidate)
        .context("dirty snapshot worker executable metadata is unavailable")?;
    validate_worker_file_metadata(&metadata)?;
    let euid = unsafe { libc::geteuid() };
    let mut component = candidate.parent();
    while let Some(directory) = component {
        let metadata = fs::symlink_metadata(directory)
            .context("dirty snapshot worker parent metadata is unavailable")?;
        let owner = metadata.uid();
        let mode = metadata.mode();
        if !metadata.file_type().is_dir() || (owner != 0 && owner != euid) || mode & 0o022 != 0 {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot worker executable path is not trusted",
            ));
        }
        component = directory.parent();
    }
    Ok(())
}

fn run_snapshot_worker_command(
    command: &mut Command,
    deadline: Instant,
) -> Result<std::process::Output> {
    output_with_aggregate_limit_until(
        command,
        deadline,
        MAX_STATE_FILE_BYTES,
        4096,
        MAX_STATE_FILE_BYTES,
    )
}

fn error_kind_from_code(code: &str) -> Option<DirtyCheckoutErrorKind> {
    [
        DirtyCheckoutErrorKind::CleanCheckout,
        DirtyCheckoutErrorKind::UnsupportedGitState,
        DirtyCheckoutErrorKind::ChallengeExpired,
        DirtyCheckoutErrorKind::ChallengeReused,
        DirtyCheckoutErrorKind::ChallengeDrift,
        DirtyCheckoutErrorKind::ForeignLease,
        DirtyCheckoutErrorKind::MalformedState,
        DirtyCheckoutErrorKind::InvalidInput,
        DirtyCheckoutErrorKind::Timeout,
        DirtyCheckoutErrorKind::ResourceUnavailable,
        DirtyCheckoutErrorKind::SnapshotFailed,
        DirtyCheckoutErrorKind::AdoptionFailed,
        DirtyCheckoutErrorKind::RevocationFailed,
    ]
    .into_iter()
    .find(|kind| kind.code() == code)
}

fn dirty_snapshot_worker(checkout: &Path, deadline: Instant) -> Result<DirtySnapshot> {
    let first = snapshot_once(checkout, deadline)?;
    let second = snapshot_once(checkout, deadline)?;
    if first.snapshot != second.snapshot {
        return Err(domain_error(
            DirtyCheckoutErrorKind::SnapshotFailed,
            "checkout changed while the dirty snapshot was calculated",
        ));
    }
    Ok(first.snapshot)
}

pub fn adopt_dirty(
    checkout: &Path,
    state_root: &Path,
    challenge_token: &str,
    reason_file: &Path,
) -> Result<AdoptionReceipt> {
    let deadline = deadline_after(SNAPSHOT_TIMEOUT)?;
    let mut previous = None;
    let mut snapshotter = |path: &Path, barrier_deadline: Instant| {
        snapshot_reusing(path, barrier_deadline, &mut previous)
    };
    let mut now = unix_time;
    adopt_dirty_inner(
        checkout,
        state_root,
        challenge_token,
        reason_file,
        deadline,
        &mut snapshotter,
        &mut now,
    )
    .map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::AdoptionFailed,
            "dirty checkout adoption failed",
        )
    })
}

fn adopt_dirty_cli(
    checkout: &Path,
    state_root: &Path,
    challenge_token: &str,
    reason_file: &Path,
) -> Result<AdoptionReceipt> {
    let deadline = deadline_after(SNAPSHOT_TIMEOUT)?;
    let mut previous: Option<DirtySnapshot> = None;
    let mut snapshotter = |path: &Path, barrier_deadline: Instant| {
        let snapshot = dirty_snapshot_cli_until(path, barrier_deadline)?;
        if previous.as_ref().is_some_and(|prior| prior != &snapshot) {
            return Err(domain_error(
                DirtyCheckoutErrorKind::SnapshotFailed,
                "checkout changed between adoption verification barriers",
            ));
        }
        previous = Some(snapshot.clone());
        Ok(snapshot)
    };
    let mut now = unix_time;
    adopt_dirty_inner(
        checkout,
        state_root,
        challenge_token,
        reason_file,
        deadline,
        &mut snapshotter,
        &mut now,
    )
    .map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::AdoptionFailed,
            "dirty checkout adoption failed",
        )
    })
}

fn snapshot_reusing(
    checkout: &Path,
    deadline: Instant,
    previous: &mut Option<DirtySnapshot>,
) -> Result<DirtySnapshot> {
    let current = dirty_snapshot_worker(checkout, deadline)?;
    if previous.as_ref().is_some_and(|prior| prior != &current) {
        return Err(domain_error(
            DirtyCheckoutErrorKind::SnapshotFailed,
            "checkout changed between adoption verification barriers",
        ));
    }
    *previous = Some(current.clone());
    Ok(current)
}

#[cfg(test)]
fn adopt_dirty_with_snapshot<F>(
    checkout: &Path,
    state_root: &Path,
    challenge_token: &str,
    reason_file: &Path,
    snapshotter: F,
) -> Result<AdoptionReceipt>
where
    F: FnMut(&Path) -> Result<DirtySnapshot>,
{
    adopt_dirty_with_snapshot_and_clock(
        checkout,
        state_root,
        challenge_token,
        reason_file,
        snapshotter,
        unix_time,
    )
}

#[cfg(test)]
fn adopt_dirty_with_snapshot_and_clock<F, G>(
    checkout: &Path,
    state_root: &Path,
    challenge_token: &str,
    reason_file: &Path,
    snapshotter: F,
    now: G,
) -> Result<AdoptionReceipt>
where
    F: FnMut(&Path) -> Result<DirtySnapshot>,
    G: FnMut() -> Result<u64>,
{
    let deadline = deadline_after(SNAPSHOT_TIMEOUT)?;
    let mut snapshotter = snapshotter;
    adopt_dirty_with_snapshot_clock_and_deadline(
        checkout,
        state_root,
        challenge_token,
        reason_file,
        |path, _| snapshotter(path),
        now,
        deadline,
    )
}

#[cfg(test)]
fn adopt_dirty_with_snapshot_clock_and_deadline<F, G>(
    checkout: &Path,
    state_root: &Path,
    challenge_token: &str,
    reason_file: &Path,
    mut snapshotter: F,
    mut now: G,
    deadline: Instant,
) -> Result<AdoptionReceipt>
where
    F: FnMut(&Path, Instant) -> Result<DirtySnapshot>,
    G: FnMut() -> Result<u64>,
{
    adopt_dirty_inner(
        checkout,
        state_root,
        challenge_token,
        reason_file,
        deadline,
        &mut snapshotter,
        &mut now,
    )
    .map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::AdoptionFailed,
            "dirty checkout adoption failed",
        )
    })
}

fn adopt_dirty_inner<F, G>(
    checkout: &Path,
    state_root: &Path,
    challenge_token: &str,
    reason_file: &Path,
    deadline: Instant,
    snapshotter: &mut F,
    now: &mut G,
) -> Result<AdoptionReceipt>
where
    F: FnMut(&Path, Instant) -> Result<DirtySnapshot>,
    G: FnMut() -> Result<u64>,
{
    if !is_lower_hex(challenge_token, 64) {
        return Err(domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "challenge token is malformed",
        ));
    }
    ensure_deadline(deadline)?;
    let token_digest = sha256_hex(challenge_token.as_bytes());
    let identity = resolve_checkout(checkout, false, deadline)?;
    let checkout_dir = checkout_state_dir(state_root, &identity)?;
    let challenge_dir = checkout_dir.join("challenges");
    verify_private_directory(&challenge_dir).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout challenge directory is untrusted",
        )
    })?;
    let _lock = LeaseLock::acquire_until(&checkout_dir, deadline)?;
    recover_checkout_pending_adoptions(
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;

    let reason = read_regular_bounded(reason_file, MAX_REASON_BYTES).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::InvalidInput,
            "adoption reason is invalid",
        )
    })?;
    let reason_text = std::str::from_utf8(&reason).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "adoption reason must be valid UTF-8",
        )
    })?;
    if reason_text.trim().is_empty() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "adoption reason must not be empty",
        ));
    }
    let reason_digest = sha256_hex(&reason);

    let pending_recovery = inspect_pending_adoption_until(
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    match pending_recovery {
        PendingRecovery::Staged | PendingRecovery::Committed => {
            let staged_pending = pending_recovery == PendingRecovery::Staged;
            return committed_adoption_retry(
                CommittedAdoptionRetry {
                    checkout_dir: &checkout_dir,
                    challenge_dir: &challenge_dir,
                    identity: &identity,
                    token_digest: &token_digest,
                    reason_digest: &reason_digest,
                    provisional_pending: true,
                    staged_pending,
                    deadline,
                },
                snapshotter,
                &mut *now,
            )?
            .ok_or_else(|| {
                domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "recovered adoption does not match its committed artifacts",
                )
            });
        }
        PendingRecovery::Revoked => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ChallengeReused,
                "dirty-checkout challenge was consumed by a revoked adoption",
            ));
        }
        PendingRecovery::None | PendingRecovery::RolledBack => {}
    }

    let challenge_path = challenge_dir.join(format!("{token_digest}.json"));
    let challenge_bytes = match read_private_regular(&challenge_path, MAX_STATE_FILE_BYTES, true) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            if let Some(receipt) = committed_adoption_retry(
                CommittedAdoptionRetry {
                    checkout_dir: &checkout_dir,
                    challenge_dir: &challenge_dir,
                    identity: &identity,
                    token_digest: &token_digest,
                    reason_digest: &reason_digest,
                    provisional_pending: false,
                    staged_pending: false,
                    deadline,
                },
                snapshotter,
                &mut *now,
            )? {
                return Ok(receipt);
            }
            return Err(domain_error(
                DirtyCheckoutErrorKind::ChallengeReused,
                "dirty-checkout challenge is unavailable or already consumed",
            ));
        }
        Err(error) => {
            return Err(command_error(
                error,
                DirtyCheckoutErrorKind::MalformedState,
                "dirty-checkout challenge state is untrusted",
            ));
        }
    };
    let challenge_digest = sha256_hex(&challenge_bytes);
    let challenge: ChallengeRecord = serde_json::from_slice(&challenge_bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout challenge state is malformed",
        )
    })?;
    validate_challenge_identity(&challenge, &identity, &token_digest)?;
    validate_challenge_at(&challenge, now()?)?;
    let initial_now = now()?;

    let lease_path = checkout_dir.join("lease.json");
    let existing_bytes = read_optional_private(&lease_path, "checkout lease")?;
    let existing = existing_bytes.as_deref().map(parse_lease).transpose()?;
    if let Some(lease) = &existing {
        validate_lease(lease, &identity)?;
        if lease.expires_at() > initial_now && lease.session_key() != challenge.session_key {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ForeignLease,
                "another session owns the active checkout lease",
            ));
        }
        if lease.expires_at() > initial_now
            && lease.session_key() == challenge.session_key
            && let Some(adoption) = lease.adoption()
        {
            if adoption.challenge_digest == challenge_digest {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::ChallengeReused,
                    "dirty-checkout challenge has already been consumed",
                ));
            }
            return Err(domain_error(
                DirtyCheckoutErrorKind::AdoptionFailed,
                "checkout already has a current-session dirty adoption",
            ));
        }
    }
    let predecessor = match existing
        .as_ref()
        .filter(|lease| lease.expires_at() <= initial_now)
    {
        Some(lease) if lease.adoption().is_some() => {
            expired_predecessor(&checkout_dir, &identity, lease)?
                .ok_or_else(|| {
                    domain_error(
                        DirtyCheckoutErrorKind::MalformedState,
                        "expired adoption predecessor state is incomplete or malformed",
                    )
                })
                .map(Some)?
        }
        _ => None,
    };

    let predecessor_lease_bytes =
        preserved_predecessor_lease_bytes(existing.as_ref(), existing_bytes.clone());

    let initial_snapshot = snapshot_before_deadline(snapshotter, &identity.root, deadline)?;
    validate_snapshot_matches_challenge(&initial_snapshot, &challenge)?;
    validate_adoption_boundary(&challenge, &identity, now()?)?;

    let receipts_dir = checkout_dir.join("receipts");
    private_directory(&receipts_dir).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty-checkout receipt directory is unavailable",
        )
    })?;
    let receipt_id = random_hex(32)?;
    let receipt_path = receipts_dir.join(format!("{receipt_id}.json"));
    let spent_challenge_path = receipts_dir.join(format!(".challenge-{receipt_id}.json"));
    let checkout_root_text = lease_path_text(&identity.root);
    let checkout_git_dir_text = lease_path_text(&identity.git_dir);
    let pending = PendingAdoptionRecord {
        schema: PENDING_ADOPTION_SCHEMA.to_string(),
        receipt_id: receipt_id.clone(),
        token_digest: token_digest.clone(),
        challenge_digest: challenge_digest.clone(),
        session_key: challenge.session_key.clone(),
        checkout_instance: identity.checkout_instance.clone(),
        snapshot_id: challenge.snapshot_id.clone(),
        predecessor_receipt_id: predecessor
            .as_ref()
            .map(|predecessor| predecessor.receipt_id.clone()),
        predecessor_receipt_digest: predecessor
            .as_ref()
            .and_then(|predecessor| predecessor.receipt_digest.clone()),
        predecessor_spent_challenge_digest: predecessor
            .as_ref()
            .and_then(|predecessor| predecessor.spent_challenge_digest.clone()),
        predecessor_lease_digest: predecessor_lease_bytes.as_deref().map(sha256_hex),
        predecessor_lease_bytes,
    };
    validate_pending_adoption(&pending, &identity, &token_digest)?;
    let pending_path = pending_adoption_path(&checkout_dir, &token_digest);
    if let Err(error) =
        write_json_atomic_with_limit(&pending_path, &pending, false, MAX_PENDING_STATE_FILE_BYTES)
    {
        return Err(transition_failure_after_recovery(
            error,
            &checkout_dir,
            &challenge_dir,
            &identity,
            &token_digest,
            deadline,
        ));
    }

    let build_transition =
        |snapshot: &DirtySnapshot, transition_at: u64| -> Result<(ReceiptRecord, LeaseRecord)> {
            let receipt = ReceiptRecord {
                schema: RECEIPT_SCHEMA.to_string(),
                receipt_id: receipt_id.clone(),
                session_key: challenge.session_key.clone(),
                repository_key: identity.repository_key.clone(),
                checkout_key: identity.checkout_key.clone(),
                checkout_instance: identity.checkout_instance.clone(),
                snapshot_id: snapshot.snapshot_id.clone(),
                authorization_turn_digest: challenge.authorization_turn_digest.clone(),
                reason_digest: reason_digest.clone(),
                challenge_digest: challenge_digest.clone(),
                adopted_at: transition_at,
            };
            let acquired_at = existing
                .as_ref()
                .filter(|lease| lease.session_key() == challenge.session_key)
                .map_or(transition_at, LeaseRecord::acquired_at);
            let lease = LeaseRecord::V2(Box::new(LeaseV2Wire {
                schema: LEASE_V2_SCHEMA.to_string(),
                session_key: challenge.session_key.clone(),
                checkout_instance: identity.checkout_instance.clone(),
                checkout_root: checkout_root_text.to_string(),
                checkout_git_dir: checkout_git_dir_text.to_string(),
                checkout_root_bytes: hex_bytes(identity.root.as_os_str().as_bytes()),
                checkout_git_dir_bytes: hex_bytes(identity.git_dir.as_os_str().as_bytes()),
                acquired_at,
                refreshed_at: transition_at,
                expires_at: transition_at.saturating_add(lease_ttl_seconds()),
                adoption: AdoptionRecord {
                    schema: ADOPTION_SCHEMA.to_string(),
                    receipt_schema: RECEIPT_SCHEMA.to_string(),
                    receipt_id: receipt_id.clone(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    authorization_turn_digest: receipt.authorization_turn_digest.clone(),
                    reason_digest: reason_digest.clone(),
                    adopted_at: transition_at,
                    challenge_issued_at: challenge.issued_at,
                    challenge_digest: challenge_digest.clone(),
                },
            }));
            validate_lease(&lease, &identity)?;
            Ok((receipt, lease))
        };

    let prepared_snapshot = transition_result_after_recovery(
        snapshot_before_deadline(snapshotter, &identity.root, deadline),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    transition_result_after_recovery(
        validate_snapshot_matches_challenge(&prepared_snapshot, &challenge),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    let prepared_at = transition_result_after_recovery(
        now(),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    transition_result_after_recovery(
        validate_adoption_boundary(&challenge, &identity, prepared_at),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    let (prepared_receipt, _) = transition_result_after_recovery(
        build_transition(&prepared_snapshot, prepared_at),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    if let Err(error) = write_json_atomic(&receipt_path, &prepared_receipt, false) {
        return Err(transition_failure_after_recovery(
            error,
            &checkout_dir,
            &challenge_dir,
            &identity,
            &token_digest,
            deadline,
        ));
    }

    let snapshot = transition_result_after_recovery(
        snapshot_before_deadline(snapshotter, &identity.root, deadline),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    transition_result_after_recovery(
        validate_snapshot_matches_challenge(&snapshot, &challenge),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    let transition_at = transition_result_after_recovery(
        now(),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    transition_result_after_recovery(
        validate_adoption_boundary(&challenge, &identity, transition_at),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    let (receipt_record, lease) = transition_result_after_recovery(
        build_transition(&snapshot, transition_at),
        &checkout_dir,
        &challenge_dir,
        &identity,
        &token_digest,
        deadline,
    )?;
    if let Err(error) = write_json_atomic(&receipt_path, &receipt_record, true) {
        return Err(transition_failure_after_recovery(
            error,
            &checkout_dir,
            &challenge_dir,
            &identity,
            &token_digest,
            deadline,
        ));
    }
    if let Err(error) =
        validate_adoption_precommit_with(&challenge, &identity, || Ok(()), &mut *now)
    {
        return Err(transition_failure_after_recovery(
            error,
            &checkout_dir,
            &challenge_dir,
            &identity,
            &token_digest,
            deadline,
        ));
    }
    if let Err(error) = fs::rename(&challenge_path, &spent_challenge_path)
        .context("failed to consume dirty-checkout challenge")
    {
        return Err(transition_failure_after_recovery(
            error,
            &checkout_dir,
            &challenge_dir,
            &identity,
            &token_digest,
            deadline,
        ));
    }
    if let Err(error) = sync_directory(&challenge_dir).and_then(|()| sync_directory(&receipts_dir))
    {
        return Err(transition_failure_after_recovery(
            error,
            &checkout_dir,
            &challenge_dir,
            &identity,
            &token_digest,
            deadline,
        ));
    }

    let staged_path = staged_lease_path(&checkout_dir, &receipt_id);
    match install_lease(&staged_path, &lease, None, &identity) {
        LeaseInstallOutcome::Installed => {
            let post_stage = snapshot_before_deadline(snapshotter, &identity.root, deadline)
                .and_then(|post_snapshot| {
                    validate_snapshot_matches_challenge(&post_snapshot, &challenge)
                })
                .and_then(|()| validate_adoption_boundary(&challenge, &identity, now()?));
            if let Err(error) = post_stage {
                if let Err(rollback_error) = rollback_provisional_adoption(
                    &staged_path,
                    &checkout_dir,
                    &challenge_dir,
                    &identity,
                    &token_digest,
                    &receipt_id,
                    deadline,
                ) {
                    return Err(domain_error(
                        DirtyCheckoutErrorKind::AdoptionFailed,
                        format!(
                            "post-stage snapshot verification failed and rollback is incomplete: {error}; rollback failed: {rollback_error}"
                        ),
                    ));
                }
                return Err(error);
            }
            match publish_staged_lease(
                &staged_path,
                &lease_path,
                &lease,
                existing_bytes.as_deref(),
                &identity,
            ) {
                LeaseInstallOutcome::Installed => {}
                LeaseInstallOutcome::NotInstalled(error) => {
                    return Err(transition_failure_after_recovery(
                        error,
                        &checkout_dir,
                        &challenge_dir,
                        &identity,
                        &token_digest,
                        deadline,
                    ));
                }
                LeaseInstallOutcome::Ambiguous(error) => {
                    return Err(domain_error(
                        DirtyCheckoutErrorKind::AdoptionFailed,
                        format!(
                            "checkout lease publication is ambiguous; recovery state was retained: {error}"
                        ),
                    ));
                }
            }
            if recover_pending_adoption_until(
                &checkout_dir,
                &challenge_dir,
                &identity,
                &token_digest,
                deadline,
            )? != PendingRecovery::Committed
            {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::AdoptionFailed,
                    "installed adoption did not enter a committed recovery state",
                ));
            }
        }
        LeaseInstallOutcome::NotInstalled(error) => {
            return Err(transition_failure_after_recovery(
                error,
                &checkout_dir,
                &challenge_dir,
                &identity,
                &token_digest,
                deadline,
            ));
        }
        LeaseInstallOutcome::Ambiguous(error) => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::AdoptionFailed,
                format!(
                    "staged checkout lease installation is ambiguous; recovery state was retained: {error}"
                ),
            ));
        }
    }

    Ok(AdoptionReceipt {
        receipt_id,
        snapshot_id: snapshot.snapshot_id,
    })
}

fn preserved_predecessor_lease_bytes(
    existing: Option<&LeaseRecord>,
    existing_bytes: Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    existing.and(existing_bytes)
}

fn snapshot_before_deadline<F>(
    snapshotter: &mut F,
    checkout: &Path,
    deadline: Instant,
) -> Result<DirtySnapshot>
where
    F: FnMut(&Path, Instant) -> Result<DirtySnapshot>,
{
    ensure_deadline(deadline)?;
    let snapshot = snapshotter(checkout, deadline)?;
    ensure_deadline(deadline)?;
    Ok(snapshot)
}

struct CommittedAdoptionRetry<'a> {
    checkout_dir: &'a Path,
    challenge_dir: &'a Path,
    identity: &'a CheckoutIdentity,
    token_digest: &'a str,
    reason_digest: &'a str,
    provisional_pending: bool,
    staged_pending: bool,
    deadline: Instant,
}

fn committed_adoption_retry<F, G>(
    retry: CommittedAdoptionRetry<'_>,
    snapshotter: &mut F,
    now: &mut G,
) -> Result<Option<AdoptionReceipt>>
where
    F: FnMut(&Path, Instant) -> Result<DirtySnapshot>,
    G: FnMut() -> Result<u64>,
{
    let CommittedAdoptionRetry {
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        reason_digest,
        provisional_pending,
        staged_pending,
        deadline,
    } = retry;
    let staged_pending_record = if staged_pending {
        let pending_path = pending_adoption_path(checkout_dir, token_digest);
        let pending_bytes =
            read_private_regular(&pending_path, MAX_PENDING_STATE_FILE_BYTES, true)?;
        let pending: PendingAdoptionRecord =
            serde_json::from_slice(&pending_bytes).map_err(|_| {
                domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "staged adoption recovery marker is malformed",
                )
            })?;
        validate_pending_adoption(&pending, identity, token_digest)?;
        Some(pending)
    } else {
        None
    };
    let authoritative_lease_path = checkout_dir.join("lease.json");
    let lease_path = staged_pending_record.as_ref().map_or_else(
        || authoritative_lease_path.clone(),
        |pending| staged_lease_path(checkout_dir, &pending.receipt_id),
    );
    let Some(lease) = load_lease(&lease_path)? else {
        return Ok(None);
    };
    validate_lease(&lease, identity)?;
    let Some(adoption) = lease.adoption() else {
        return Ok(None);
    };
    let receipt_id = adoption.receipt_id.clone();
    let receipts_dir = checkout_dir.join("receipts");
    verify_private_directory(&receipts_dir).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            "committed adoption receipt directory is untrusted",
        )
    })?;
    let spent_path = receipts_dir.join(format!(".challenge-{receipt_id}.json"));
    let Some(spent_bytes) = read_optional_private(&spent_path, "committed adoption challenge")?
    else {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "committed adoption challenge artifact is missing",
        ));
    };
    let spent: ChallengeRecord = serde_json::from_slice(&spent_bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "committed adoption challenge artifact is malformed",
        )
    })?;
    validate_challenge_identity(&spent, identity, &spent.token_digest)?;
    if spent.token_digest != token_digest {
        return Ok(None);
    }
    let receipt_path = receipts_dir.join(format!("{receipt_id}.json"));
    let receipt_bytes =
        read_private_regular(&receipt_path, MAX_STATE_FILE_BYTES, true).map_err(|error| {
            command_error(
                error,
                DirtyCheckoutErrorKind::MalformedState,
                "committed adoption receipt is unavailable",
            )
        })?;
    let receipt: ReceiptRecord = serde_json::from_slice(&receipt_bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "committed adoption receipt is malformed",
        )
    })?;
    validate_receipt(&receipt, identity, &receipt_id)?;
    validate_receipt_matches_lease(&receipt, &lease)?;
    validate_spent_challenge_matches_lease(&spent_bytes, &spent, identity, &lease)?;
    if adoption.reason_digest != reason_digest {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ChallengeReused,
            "committed adoption retry inputs or artifacts do not match",
        ));
    }

    let verification = (|| -> Result<()> {
        let before_snapshot = now()?;
        if lease.expires_at() <= before_snapshot {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ChallengeReused,
                "committed adoption lease expired during retry validation",
            ));
        }
        if provisional_pending {
            validate_adoption_boundary(&spent, identity, before_snapshot)?;
        }
        let snapshot = snapshot_before_deadline(snapshotter, &identity.root, deadline)?;
        validate_snapshot_matches_challenge(&snapshot, &spent)?;
        let after_snapshot = now()?;
        if lease.expires_at() <= after_snapshot {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ChallengeReused,
                "committed adoption lease expired during retry validation",
            ));
        }
        if provisional_pending {
            validate_adoption_boundary(&spent, identity, after_snapshot)?;
        }
        Ok(())
    })();

    if let Err(error) = verification {
        if provisional_pending
            && let Err(rollback_error) = rollback_provisional_adoption(
                &lease_path,
                checkout_dir,
                challenge_dir,
                identity,
                token_digest,
                &receipt_id,
                deadline,
            )
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::AdoptionFailed,
                format!(
                    "provisional adoption verification failed and rollback is incomplete: {error}; rollback failed: {rollback_error}"
                ),
            ));
        }
        return Err(error);
    }

    if let Some(pending) = &staged_pending_record {
        match publish_staged_lease(
            &lease_path,
            &authoritative_lease_path,
            &lease,
            pending.predecessor_lease_bytes.as_deref(),
            identity,
        ) {
            LeaseInstallOutcome::Installed => {}
            LeaseInstallOutcome::NotInstalled(error) => return Err(error),
            LeaseInstallOutcome::Ambiguous(error) => {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::AdoptionFailed,
                    format!(
                        "checkout lease publication is ambiguous; recovery state was retained: {error}"
                    ),
                ));
            }
        }
    }

    if provisional_pending
        && recover_pending_adoption_until(
            checkout_dir,
            challenge_dir,
            identity,
            token_digest,
            deadline,
        )? != PendingRecovery::Committed
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::AdoptionFailed,
            "verified provisional adoption did not finalize committed recovery",
        ));
    }
    Ok(Some(AdoptionReceipt {
        receipt_id: receipt.receipt_id,
        snapshot_id: receipt.snapshot_id,
    }))
}

fn validate_snapshot_matches_challenge(
    snapshot: &DirtySnapshot,
    challenge: &ChallengeRecord,
) -> Result<()> {
    if snapshot.repository_key != challenge.repository_key
        || snapshot.checkout_key != challenge.checkout_key
        || snapshot.checkout_instance != challenge.checkout_instance
        || snapshot.snapshot_id != challenge.snapshot_id
        || snapshot.head_oid != challenge.head_oid
        || snapshot.branch_ref_digest != challenge.branch_ref_digest
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ChallengeDrift,
            "dirty-checkout challenge no longer matches the current checkout snapshot",
        ));
    }
    Ok(())
}

fn rollback_provisional_adoption(
    provisional_lease_path: &Path,
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    receipt_id: &str,
    deadline: Instant,
) -> Result<()> {
    rollback_provisional_adoption_with_pruning(ProvisionalRollback {
        provisional_lease_path,
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        receipt_id,
        prune_revocations: true,
        deadline,
    })
}

fn rollback_provisional_adoption_without_pruning(
    provisional_lease_path: &Path,
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    receipt_id: &str,
    deadline: Instant,
) -> Result<()> {
    rollback_provisional_adoption_with_pruning(ProvisionalRollback {
        provisional_lease_path,
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        receipt_id,
        prune_revocations: false,
        deadline,
    })
}

struct ProvisionalRollback<'a> {
    provisional_lease_path: &'a Path,
    checkout_dir: &'a Path,
    challenge_dir: &'a Path,
    identity: &'a CheckoutIdentity,
    token_digest: &'a str,
    receipt_id: &'a str,
    prune_revocations: bool,
    deadline: Instant,
}

fn rollback_provisional_adoption_with_pruning(rollback: ProvisionalRollback<'_>) -> Result<()> {
    let ProvisionalRollback {
        provisional_lease_path,
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        receipt_id,
        prune_revocations,
        deadline,
    } = rollback;
    let revoked_lease_path = checkout_dir.join(format!(".revoked-{receipt_id}.json"));
    fs::rename(provisional_lease_path, &revoked_lease_path)
        .context("provisional adoption rollback could not revoke the lease")?;
    sync_directory(checkout_dir)
        .context("provisional adoption rollback tombstone could not be made durable")?;
    let recovery = if prune_revocations {
        recover_pending_adoption_until(
            checkout_dir,
            challenge_dir,
            identity,
            token_digest,
            deadline,
        )?
    } else {
        recover_pending_adoption_without_pruning(
            checkout_dir,
            challenge_dir,
            identity,
            token_digest,
            deadline,
        )?
    };
    if recovery != PendingRecovery::Revoked {
        return Err(domain_error(
            DirtyCheckoutErrorKind::AdoptionFailed,
            "provisional adoption rollback did not enter revoked recovery state",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct ExpiredPredecessor {
    receipt_id: String,
    receipt_path: PathBuf,
    spent_challenge_path: PathBuf,
    receipt_digest: Option<String>,
    spent_challenge_digest: Option<String>,
}

fn expired_predecessor(
    checkout_dir: &Path,
    identity: &CheckoutIdentity,
    lease: &LeaseRecord,
) -> Result<Option<ExpiredPredecessor>> {
    let Some(adoption) = lease.adoption() else {
        return Ok(None);
    };
    let receipts_dir = checkout_dir.join("receipts");
    let receipt_path = receipts_dir.join(format!("{}.json", adoption.receipt_id));
    let spent_challenge_path =
        receipts_dir.join(format!(".challenge-{}.json", adoption.receipt_id));
    let receipt_bytes =
        read_optional_private(&receipt_path, "expired adoption predecessor receipt")?;
    if let Some(receipt_bytes) = &receipt_bytes {
        let receipt: ReceiptRecord = serde_json::from_slice(receipt_bytes).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "expired adoption predecessor receipt is malformed",
            )
        })?;
        validate_receipt(&receipt, identity, &adoption.receipt_id)?;
        validate_receipt_matches_lease(&receipt, lease)?;
    }
    let spent_bytes = read_optional_private(
        &spent_challenge_path,
        "expired adoption predecessor challenge",
    )?;
    if let Some(spent_bytes) = &spent_bytes {
        let spent: ChallengeRecord = serde_json::from_slice(spent_bytes).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "expired adoption predecessor challenge is malformed",
            )
        })?;
        validate_spent_challenge_matches_lease(spent_bytes, &spent, identity, lease)?;
    }
    Ok(Some(ExpiredPredecessor {
        receipt_id: adoption.receipt_id.clone(),
        receipt_path,
        spent_challenge_path,
        receipt_digest: receipt_bytes.as_deref().map(sha256_hex),
        spent_challenge_digest: spent_bytes.as_deref().map(sha256_hex),
    }))
}

fn load_bound_predecessor(
    receipts_dir: &Path,
    receipt_id: &str,
    expected_receipt_digest: Option<&str>,
    expected_spent_challenge_digest: Option<&str>,
) -> Result<ExpiredPredecessor> {
    let receipt_path = receipts_dir.join(format!("{receipt_id}.json"));
    let spent_challenge_path = receipts_dir.join(format!(".challenge-{receipt_id}.json"));
    let receipt_bytes = read_optional_private(&receipt_path, "bound predecessor receipt")?;
    let spent_bytes = read_optional_private(&spent_challenge_path, "bound predecessor challenge")?;
    let receipt_matches = match (&receipt_bytes, expected_receipt_digest) {
        (Some(bytes), Some(expected)) => sha256_hex(bytes) == expected,
        (None, _) => true,
        (Some(_), None) => false,
    };
    let spent_matches = match (&spent_bytes, expected_spent_challenge_digest) {
        (Some(bytes), Some(expected)) => sha256_hex(bytes) == expected,
        (None, _) => true,
        (Some(_), None) => false,
    };
    if !receipt_matches || !spent_matches {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "bound predecessor artifacts do not match pending adoption state",
        ));
    }
    Ok(ExpiredPredecessor {
        receipt_id: receipt_id.to_string(),
        receipt_path,
        spent_challenge_path,
        receipt_digest: expected_receipt_digest.map(str::to_string),
        spent_challenge_digest: expected_spent_challenge_digest.map(str::to_string),
    })
}

fn cleanup_expired_predecessor(predecessor: &ExpiredPredecessor) -> Result<()> {
    for path in [&predecessor.receipt_path, &predecessor.spent_challenge_path] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("expired adoption predecessor cleanup failed");
            }
        }
    }
    if let Some(parent) = predecessor.receipt_path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

struct RecoveryScanBudget {
    deadline: Instant,
    directory_entries: usize,
    state_entries: usize,
    aggregate_bytes: u64,
    name_bytes: usize,
}

impl RecoveryScanBudget {
    fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            directory_entries: 0,
            state_entries: 0,
            aggregate_bytes: 0,
            name_bytes: 0,
        }
    }

    fn charge(&mut self, entry: &fs::DirEntry, state_entry: bool) -> Result<()> {
        ensure_deadline(self.deadline)?;
        self.directory_entries = self.directory_entries.saturating_add(1);
        self.name_bytes = self
            .name_bytes
            .saturating_add(entry.file_name().as_bytes().len());
        if self.directory_entries > MAX_RECOVERY_DIRECTORY_ENTRIES
            || self.name_bytes > MAX_RECOVERY_NAME_BYTES
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "checkout recovery directory exceeds the supported scan limit",
            ));
        }
        if state_entry {
            self.state_entries = self.state_entries.saturating_add(1);
            let metadata = fs::symlink_metadata(entry.path())
                .context("checkout recovery state metadata is unavailable")?;
            self.aggregate_bytes = self.aggregate_bytes.saturating_add(metadata.len());
            if self.state_entries > MAX_RECOVERY_STATE_ENTRIES
                || self.aggregate_bytes > MAX_RECOVERY_AGGREGATE_BYTES
            {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::ResourceUnavailable,
                    "checkout recovery state exceeds the supported aggregate limit",
                ));
            }
        }
        Ok(())
    }

    fn check_deadline(&self) -> Result<()> {
        ensure_deadline(self.deadline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRecovery {
    None,
    RolledBack,
    Staged,
    Committed,
    Revoked,
}

fn pending_adoption_path(checkout_dir: &Path, token_digest: &str) -> PathBuf {
    checkout_dir.join(format!(".pending-adoption-{token_digest}.json"))
}

fn staged_lease_path(checkout_dir: &Path, receipt_id: &str) -> PathBuf {
    checkout_dir.join(format!(".pending-lease-{receipt_id}.json"))
}

fn recover_pending_batch_with<I, R, B, P>(
    records: I,
    current_token_digest: &str,
    mut recover: R,
    mut rollback_staged: B,
    prune: P,
) -> Result<()>
where
    I: IntoIterator<Item = (String, String)>,
    R: FnMut(&str) -> Result<PendingRecovery>,
    B: FnMut(&str, &str) -> Result<()>,
    P: FnOnce(Option<&str>) -> Result<()>,
{
    let mut failed_receipt_id = None;
    let recovery_result = (|| {
        for (token_digest, receipt_id) in records {
            if token_digest == current_token_digest {
                continue;
            }
            let recovery = match recover(&token_digest) {
                Ok(recovery) => recovery,
                Err(error) => {
                    failed_receipt_id = Some(receipt_id);
                    return Err(error);
                }
            };
            if recovery == PendingRecovery::Staged
                && let Err(error) = rollback_staged(&token_digest, &receipt_id)
            {
                failed_receipt_id = Some(receipt_id);
                return Err(error);
            }
        }
        Ok(())
    })();
    let prune_result = prune(failed_receipt_id.as_deref());

    match (recovery_result, prune_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(prune_error)) => Err(prune_error),
        (Err(primary), Err(prune_error)) => {
            let context = format!(
                "checkout-wide pending-adoption recovery failed: {primary:#}; deferred revocation tombstone pruning also failed: {prune_error:#}"
            );
            Err(primary.context(context))
        }
    }
}

fn recover_checkout_pending_adoptions(
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    current_token_digest: &str,
    deadline: Instant,
) -> Result<()> {
    let mut budget = RecoveryScanBudget::new(deadline);
    let mut pending_records = Vec::new();
    let mut protected_receipts = HashSet::new();
    let mut tombstone_candidates = HashSet::new();
    for entry in fs::read_dir(checkout_dir).context("pending adoption scan is unavailable")? {
        let entry = entry.context("pending adoption scan entry is unreadable")?;
        let name = entry.file_name();
        let token_digest = name
            .to_str()
            .and_then(|name| name.strip_prefix(".pending-adoption-"))
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|digest| is_lower_hex(digest, 64));
        let tombstone_receipt = name
            .to_str()
            .and_then(|name| name.strip_prefix(".revoked-"))
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|receipt_id| is_lower_hex(receipt_id, 64));
        budget.charge(
            &entry,
            token_digest.is_some() || tombstone_receipt.is_some(),
        )?;
        if let Some(receipt_id) = tombstone_receipt {
            tombstone_candidates.insert(receipt_id.to_string());
        }
        let Some(token_digest) = token_digest else {
            continue;
        };
        let pending_bytes =
            read_private_regular(&entry.path(), MAX_PENDING_STATE_FILE_BYTES, true)?;
        let pending: PendingAdoptionRecord =
            serde_json::from_slice(&pending_bytes).map_err(|_| {
                domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "checkout-wide recovery marker is malformed",
                )
            })?;
        validate_pending_adoption(&pending, identity, token_digest)?;
        tombstone_candidates.insert(pending.receipt_id.clone());
        protected_receipts.insert(pending.receipt_id.clone());
        pending_records.push((token_digest.to_string(), pending.receipt_id));
    }
    budget.check_deadline()?;

    recover_pending_batch_with(
        pending_records,
        current_token_digest,
        |token_digest| {
            recover_pending_adoption_without_pruning(
                checkout_dir,
                challenge_dir,
                identity,
                token_digest,
                deadline,
            )
        },
        |token_digest, receipt_id| {
            rollback_provisional_adoption_without_pruning(
                &staged_lease_path(checkout_dir, receipt_id),
                checkout_dir,
                challenge_dir,
                identity,
                token_digest,
                receipt_id,
                deadline,
            )
        },
        |failed_receipt_id| {
            if let Some(receipt_id) = failed_receipt_id {
                protected_receipts.insert(receipt_id.to_string());
            }
            prune_revocation_tombstone_candidates(
                checkout_dir,
                identity,
                &protected_receipts,
                tombstone_candidates,
                deadline,
            )
        },
    )?;
    budget.check_deadline()
}

#[cfg(test)]
fn recover_pending_adoption(
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
) -> Result<PendingRecovery> {
    recover_pending_adoption_until(
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        deadline_after(SNAPSHOT_TIMEOUT)?,
    )
}

fn recover_pending_adoption_until(
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    deadline: Instant,
) -> Result<PendingRecovery> {
    recover_pending_adoption_with(
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        true,
        true,
        deadline,
    )
}

fn recover_pending_adoption_without_pruning(
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    deadline: Instant,
) -> Result<PendingRecovery> {
    recover_pending_adoption_with(
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        true,
        false,
        deadline,
    )
}

fn inspect_pending_adoption_until(
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    deadline: Instant,
) -> Result<PendingRecovery> {
    recover_pending_adoption_with(
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        false,
        false,
        deadline,
    )
}

fn recover_pending_adoption_with(
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    finalize_committed: bool,
    prune_revocations: bool,
    deadline: Instant,
) -> Result<PendingRecovery> {
    ensure_deadline(deadline)?;
    let pending_path = pending_adoption_path(checkout_dir, token_digest);
    let Some(pending_bytes) = read_optional_private_with_limit(
        &pending_path,
        "pending adoption",
        MAX_PENDING_STATE_FILE_BYTES,
    )?
    else {
        return Ok(PendingRecovery::None);
    };
    let pending: PendingAdoptionRecord = serde_json::from_slice(&pending_bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "pending dirty-checkout adoption state is malformed",
        )
    })?;
    validate_pending_adoption(&pending, identity, token_digest)?;

    let receipts_dir = checkout_dir.join("receipts");
    verify_private_directory(&receipts_dir).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            "pending adoption receipt directory is untrusted",
        )
    })?;
    let receipt_path = receipts_dir.join(format!("{}.json", pending.receipt_id));
    let spent_challenge_path = receipts_dir.join(format!(".challenge-{}.json", pending.receipt_id));
    let challenge_path = challenge_dir.join(format!("{token_digest}.json"));
    let revoked_lease_path = checkout_dir.join(format!(".revoked-{}.json", pending.receipt_id));

    if let Some(revoked_bytes) = read_optional_private(&revoked_lease_path, "revoked adoption")? {
        let revoked = parse_lease(&revoked_bytes)?;
        validate_lease(&revoked, identity)?;
        let adoption = revoked.adoption().ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revoked adoption tombstone has no adoption identity",
            )
        })?;
        if revoked.session_key() != pending.session_key
            || revoked.checkout_instance() != pending.checkout_instance
            || adoption.receipt_id != pending.receipt_id
            || adoption.snapshot_id != pending.snapshot_id
            || adoption.challenge_digest != pending.challenge_digest
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revoked adoption tombstone does not match pending state",
            ));
        }
        if let Some(challenge) = load_pending_challenge(
            &challenge_path,
            identity,
            token_digest,
            &pending.challenge_digest,
        )? && challenge.token_digest != pending.token_digest
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revoked adoption has an inconsistent live challenge",
            ));
        }
        if let Some(spent) = load_pending_challenge(
            &spent_challenge_path,
            identity,
            token_digest,
            &pending.challenge_digest,
        )? && (spent.token_digest != pending.token_digest
            || spent.session_key != revoked.session_key()
            || spent.snapshot_id != pending.snapshot_id
            || spent.authorization_turn_digest != adoption.authorization_turn_digest
            || spent.issued_at != adoption.challenge_issued_at
            || adoption.adopted_at < spent.issued_at
            || adoption.adopted_at >= spent.expires_at)
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revoked adoption has an inconsistent spent challenge",
            ));
        }
        if let Some(receipt_bytes) =
            read_optional_private(&receipt_path, "revoked adoption receipt")?
        {
            let receipt: ReceiptRecord = serde_json::from_slice(&receipt_bytes).map_err(|_| {
                domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "revoked adoption receipt state is malformed",
                )
            })?;
            validate_receipt(&receipt, identity, &pending.receipt_id)?;
            validate_receipt_matches_lease(&receipt, &revoked)?;
        }
        restore_pending_predecessor_lease(checkout_dir, &pending, identity)?;
        for path in [&challenge_path, &spent_challenge_path, &receipt_path] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context("revoked adoption recovery cleanup failed");
                }
            }
        }
        sync_directory(challenge_dir)?;
        sync_directory(&receipts_dir)?;
        if prune_revocations {
            prune_revocation_tombstones(checkout_dir, identity, &pending.receipt_id, deadline)?;
        }
        fs::remove_file(&pending_path)
            .context("revoked adoption recovery marker cleanup failed")?;
        sync_directory(checkout_dir)?;
        return Ok(PendingRecovery::Revoked);
    }

    let staged_path = staged_lease_path(checkout_dir, &pending.receipt_id);
    if let Some(staged_bytes) = read_optional_private(&staged_path, "staged adoption lease")? {
        let staged = parse_lease(&staged_bytes)?;
        validate_lease(&staged, identity)?;
        let adoption = staged.adoption().ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "staged adoption lease has no adoption identity",
            )
        })?;
        if staged.session_key() != pending.session_key
            || staged.checkout_instance() != pending.checkout_instance
            || adoption.receipt_id != pending.receipt_id
            || adoption.snapshot_id != pending.snapshot_id
            || adoption.challenge_digest != pending.challenge_digest
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "staged adoption lease does not match pending state",
            ));
        }
        if load_pending_challenge(
            &challenge_path,
            identity,
            token_digest,
            &pending.challenge_digest,
        )?
        .is_some()
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "staged adoption has an inconsistent live challenge",
            ));
        }
        let artifacts = expired_predecessor(checkout_dir, identity, &staged)?.ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "staged pending adoption artifacts are incomplete or malformed",
            )
        })?;
        if artifacts.receipt_id != pending.receipt_id
            || artifacts.receipt_digest.is_none()
            || artifacts.spent_challenge_digest.is_none()
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "staged pending adoption artifacts are incomplete or inconsistent",
            ));
        }
        return Ok(PendingRecovery::Staged);
    }

    let lease = load_lease(&checkout_dir.join("lease.json"))?;
    if let Some(lease) = &lease {
        validate_lease(lease, identity)?;
    }
    let committed = lease.as_ref().is_some_and(|lease| {
        lease.session_key() == pending.session_key
            && lease.checkout_instance() == pending.checkout_instance
            && lease.adoption().is_some_and(|adoption| {
                adoption.receipt_id == pending.receipt_id
                    && adoption.snapshot_id == pending.snapshot_id
                    && adoption.challenge_digest == pending.challenge_digest
            })
    });
    if committed {
        let lease = lease.expect("checked committed lease");
        validate_lease(&lease, identity)?;
        let artifacts = expired_predecessor(checkout_dir, identity, &lease)?.ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "committed pending adoption artifacts are incomplete or malformed",
            )
        })?;
        if artifacts.receipt_id != pending.receipt_id
            || artifacts.receipt_digest.is_none()
            || artifacts.spent_challenge_digest.is_none()
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "committed pending adoption artifacts are incomplete or inconsistent",
            ));
        }
        if !finalize_committed {
            return Ok(PendingRecovery::Committed);
        }
        if let Some(predecessor_receipt_id) = &pending.predecessor_receipt_id {
            let predecessor = load_bound_predecessor(
                &receipts_dir,
                predecessor_receipt_id,
                pending.predecessor_receipt_digest.as_deref(),
                pending.predecessor_spent_challenge_digest.as_deref(),
            )?;
            cleanup_expired_predecessor(&predecessor)?;
        }
        fs::remove_file(&pending_path)
            .context("committed adoption recovery marker cleanup failed")?;
        sync_directory(checkout_dir)?;
        return Ok(PendingRecovery::Committed);
    }

    let current_challenge = load_pending_challenge(
        &challenge_path,
        identity,
        token_digest,
        &pending.challenge_digest,
    )?;
    let spent_challenge = load_pending_challenge(
        &spent_challenge_path,
        identity,
        token_digest,
        &pending.challenge_digest,
    )?;
    match (current_challenge.is_some(), spent_challenge.is_some()) {
        (true, false) => {}
        (false, true) => fs::rename(&spent_challenge_path, &challenge_path)
            .context("pending adoption challenge rollback failed")?,
        (true, true) => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "pending adoption has duplicate current and spent challenges",
            ));
        }
        (false, false) => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "pending adoption challenge state is missing",
            ));
        }
    }

    if let Some(receipt_bytes) = read_optional_private(&receipt_path, "pending adoption receipt")? {
        let receipt: ReceiptRecord = serde_json::from_slice(&receipt_bytes).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "pending adoption receipt state is malformed",
            )
        })?;
        validate_receipt(&receipt, identity, &pending.receipt_id)?;
        if receipt.session_key != pending.session_key
            || receipt.snapshot_id != pending.snapshot_id
            || receipt.challenge_digest != pending.challenge_digest
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "pending adoption receipt does not match its recovery marker",
            ));
        }
        fs::remove_file(&receipt_path).context("pending adoption receipt rollback failed")?;
    }
    sync_directory(challenge_dir)?;
    sync_directory(&receipts_dir)?;
    fs::remove_file(&pending_path).context("pending adoption recovery marker cleanup failed")?;
    sync_directory(checkout_dir)?;
    Ok(PendingRecovery::RolledBack)
}

fn read_optional_private(path: &Path, label: &str) -> Result<Option<Vec<u8>>> {
    read_optional_private_with_limit(path, label, MAX_STATE_FILE_BYTES)
}

fn read_optional_private_with_limit(
    path: &Path,
    label: &str,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>> {
    match read_private_regular(path, max_bytes, true) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            format!("{label} state is untrusted"),
        )),
    }
}

fn load_pending_challenge(
    path: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    challenge_digest: &str,
) -> Result<Option<ChallengeRecord>> {
    let Some(bytes) = read_optional_private(path, "pending adoption challenge")? else {
        return Ok(None);
    };
    if sha256_hex(&bytes) != challenge_digest {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "pending adoption challenge artifact does not match its recovery marker",
        ));
    }
    let challenge: ChallengeRecord = serde_json::from_slice(&bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "pending adoption challenge state is malformed",
        )
    })?;
    validate_challenge_identity(&challenge, identity, token_digest)?;
    Ok(Some(challenge))
}

fn validate_pending_adoption(
    pending: &PendingAdoptionRecord,
    identity: &CheckoutIdentity,
    token_digest: &str,
) -> Result<()> {
    let valid = pending.schema == PENDING_ADOPTION_SCHEMA
        && is_lower_hex(&pending.receipt_id, 64)
        && is_lower_hex(&pending.token_digest, 64)
        && is_lower_hex(&pending.challenge_digest, 64)
        && is_lower_hex(&pending.session_key, 64)
        && is_lower_hex(&pending.checkout_instance, 32)
        && is_lower_hex(&pending.snapshot_id, 64)
        && pending.token_digest == token_digest
        && pending.checkout_instance == identity.checkout_instance
        && match (
            &pending.predecessor_receipt_id,
            &pending.predecessor_receipt_digest,
            &pending.predecessor_spent_challenge_digest,
        ) {
            (None, None, None) => true,
            (Some(receipt_id), receipt_digest, spent_digest) => {
                is_lower_hex(receipt_id, 64)
                    && receipt_id != &pending.receipt_id
                    && receipt_digest
                        .as_ref()
                        .is_none_or(|digest| is_lower_hex(digest, 64))
                    && spent_digest
                        .as_ref()
                        .is_none_or(|digest| is_lower_hex(digest, 64))
            }
            (None, _, _) => false,
        };
    if !valid {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "pending dirty-checkout adoption state is malformed",
        ));
    }
    validate_pending_predecessor_lease(pending, identity)?;
    Ok(())
}

fn validate_pending_predecessor_lease<'a>(
    pending: &'a PendingAdoptionRecord,
    identity: &CheckoutIdentity,
) -> Result<Option<&'a [u8]>> {
    let (digest, bytes) = match (
        pending.predecessor_lease_digest.as_deref(),
        pending.predecessor_lease_bytes.as_deref(),
    ) {
        (None, None) => return Ok(None),
        (Some(digest), Some(bytes)) => (digest, bytes),
        _ => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "pending predecessor lease binding is incomplete",
            ));
        }
    };
    let lease = parse_lease(bytes)?;
    validate_lease(&lease, identity)?;
    let predecessor_matches = match (lease.adoption(), pending.predecessor_receipt_id.as_deref()) {
        (None, None) => lease.schema() == LEASE_V1_SCHEMA,
        (Some(adoption), Some(receipt_id)) => adoption.receipt_id == receipt_id,
        _ => false,
    };
    if !is_lower_hex(digest, 64)
        || sha256_hex(bytes) != digest
        || lease.checkout_instance() != pending.checkout_instance
        || !predecessor_matches
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "pending predecessor lease does not match its recovery marker",
        ));
    }
    Ok(Some(bytes))
}

fn restore_pending_predecessor_lease(
    checkout_dir: &Path,
    pending: &PendingAdoptionRecord,
    identity: &CheckoutIdentity,
) -> Result<()> {
    let expected = validate_pending_predecessor_lease(pending, identity)?;
    let lease_path = checkout_dir.join("lease.json");
    match (
        read_optional_private(&lease_path, "restored predecessor lease")?,
        expected,
    ) {
        (None, Some(bytes)) => write_state_atomic(&lease_path, bytes, false)
            .context("exact predecessor lease restoration failed"),
        (Some(actual), Some(expected)) if actual == expected => Ok(()),
        (None, None) => Ok(()),
        _ => Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "revoked adoption predecessor lease state is inconsistent",
        )),
    }
}

fn transition_result_after_recovery<T>(
    result: Result<T>,
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    deadline: Instant,
) -> Result<T> {
    result.map_err(|error| {
        transition_failure_after_recovery(
            error,
            checkout_dir,
            challenge_dir,
            identity,
            token_digest,
            deadline,
        )
    })
}

fn transition_failure_after_recovery(
    error: anyhow::Error,
    checkout_dir: &Path,
    challenge_dir: &Path,
    identity: &CheckoutIdentity,
    token_digest: &str,
    deadline: Instant,
) -> anyhow::Error {
    match inspect_pending_adoption_until(
        checkout_dir,
        challenge_dir,
        identity,
        token_digest,
        deadline,
    ) {
        Ok(PendingRecovery::None | PendingRecovery::RolledBack) => error,
        Ok(PendingRecovery::Staged | PendingRecovery::Committed | PendingRecovery::Revoked) => {
            domain_error(
                DirtyCheckoutErrorKind::AdoptionFailed,
                format!(
                    "adoption transition retained recovery state while handling a failure: {error}"
                ),
            )
        }
        Err(recovery_error) => domain_error(
            DirtyCheckoutErrorKind::AdoptionFailed,
            format!(
                "adoption transition failed and requires recovery: {error}; recovery failed: {recovery_error}"
            ),
        ),
    }
}
fn publish_staged_lease(
    staged_path: &Path,
    authoritative_path: &Path,
    expected: &LeaseRecord,
    expected_previous_bytes: Option<&[u8]>,
    identity: &CheckoutIdentity,
) -> LeaseInstallOutcome {
    let actual_previous_bytes =
        match read_optional_private(authoritative_path, "authoritative lease") {
            Ok(actual) => actual,
            Err(error) => return LeaseInstallOutcome::NotInstalled(error),
        };
    if actual_previous_bytes.as_deref() != expected_previous_bytes {
        let error = match actual_previous_bytes.as_deref() {
            Some(bytes) => {
                match parse_lease(bytes).and_then(|lease| validate_lease(&lease, identity)) {
                    Ok(()) => domain_error(
                        DirtyCheckoutErrorKind::ForeignLease,
                        "authoritative checkout lease changed before staged publication",
                    ),
                    Err(error) => error,
                }
            }
            None => domain_error(
                DirtyCheckoutErrorKind::AdoptionFailed,
                "authoritative checkout lease disappeared before staged publication",
            ),
        };
        return LeaseInstallOutcome::NotInstalled(error);
    }
    let previous = match expected_previous_bytes.map(parse_lease).transpose() {
        Ok(previous) => previous,
        Err(error) => return LeaseInstallOutcome::NotInstalled(error),
    };
    install_lease_with(
        authoritative_path,
        expected,
        previous.as_ref(),
        identity,
        || {
            let staged = load_lease(staged_path)?.ok_or_else(|| {
                domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "staged checkout lease is missing",
                )
            })?;
            validate_lease(&staged, identity)?;
            ensure!(
                staged == *expected,
                "staged checkout lease changed before publication"
            );
            fs::rename(staged_path, authoritative_path)
                .context("staged checkout lease could not be published")?;
            sync_directory(
                authoritative_path
                    .parent()
                    .context("checkout lease has no parent directory")?,
            )
        },
    )
}

enum LeaseInstallOutcome {
    Installed,
    NotInstalled(anyhow::Error),
    Ambiguous(anyhow::Error),
}

fn install_lease(
    path: &Path,
    expected: &LeaseRecord,
    previous: Option<&LeaseRecord>,
    identity: &CheckoutIdentity,
) -> LeaseInstallOutcome {
    install_lease_with(path, expected, previous, identity, || {
        write_json_atomic(path, expected, true)
    })
}

fn install_lease_with<F>(
    path: &Path,
    expected: &LeaseRecord,
    previous: Option<&LeaseRecord>,
    identity: &CheckoutIdentity,
    writer: F,
) -> LeaseInstallOutcome
where
    F: FnOnce() -> Result<()>,
{
    let Err(error) = writer() else {
        return LeaseInstallOutcome::Installed;
    };
    match load_lease(path) {
        Ok(Some(actual)) if validate_lease(&actual, identity).is_ok() && actual == *expected => {
            LeaseInstallOutcome::Installed
        }
        Ok(actual) if actual.as_ref() == previous => LeaseInstallOutcome::NotInstalled(error),
        _ => LeaseInstallOutcome::Ambiguous(error),
    }
}

pub fn revoke_dirty(checkout: &Path, state_root: &Path, receipt_id: &str) -> Result<()> {
    revoke_dirty_inner(checkout, state_root, receipt_id).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::RevocationFailed,
            "dirty checkout revocation failed",
        )
    })
}

fn revoke_dirty_inner(checkout: &Path, state_root: &Path, receipt_id: &str) -> Result<()> {
    revoke_dirty_inner_until(
        checkout,
        state_root,
        receipt_id,
        deadline_after(SNAPSHOT_TIMEOUT)?,
    )
}

fn revoke_dirty_inner_until(
    checkout: &Path,
    state_root: &Path,
    receipt_id: &str,
    deadline: Instant,
) -> Result<()> {
    if !is_lower_hex(receipt_id, 64) {
        return Err(domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "receipt ID is malformed",
        ));
    }
    let identity = resolve_checkout(checkout, false, deadline)?;
    let checkout_dir = checkout_state_dir(state_root, &identity)?;
    let receipts_dir = checkout_dir.join("receipts");
    verify_private_directory(&receipts_dir).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout receipt directory is untrusted",
        )
    })?;
    let challenge_dir = checkout_dir.join("challenges");
    verify_private_directory(&challenge_dir).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout challenge directory is untrusted",
        )
    })?;
    let _lock = LeaseLock::acquire_until(&checkout_dir, deadline)?;
    if recover_revocation_tombstone(
        &checkout_dir,
        &challenge_dir,
        &receipts_dir,
        &identity,
        receipt_id,
        deadline,
    )? {
        return Ok(());
    }

    let receipt_path = receipts_dir.join(format!("{receipt_id}.json"));
    let receipt_bytes =
        read_private_regular(&receipt_path, MAX_STATE_FILE_BYTES, true).map_err(|error| {
            command_error(
                error,
                DirtyCheckoutErrorKind::MalformedState,
                "dirty-checkout receipt state is untrusted",
            )
        })?;
    let receipt: ReceiptRecord = serde_json::from_slice(&receipt_bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout receipt state is malformed",
        )
    })?;
    validate_receipt(&receipt, &identity, receipt_id)?;

    let lease_path = checkout_dir.join("lease.json");
    let lease = load_lease(&lease_path)?.ok_or_else(|| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout lease is missing",
        )
    })?;
    validate_lease(&lease, &identity)?;
    validate_receipt_matches_lease(&receipt, &lease)?;

    let spent_path = receipts_dir.join(format!(".challenge-{receipt_id}.json"));
    let spent_bytes =
        read_private_regular(&spent_path, MAX_STATE_FILE_BYTES, true).map_err(|error| {
            command_error(
                error,
                DirtyCheckoutErrorKind::MalformedState,
                "dirty-checkout spent challenge is unavailable",
            )
        })?;
    let spent: ChallengeRecord = serde_json::from_slice(&spent_bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout spent challenge is malformed",
        )
    })?;
    validate_spent_challenge_matches_lease(&spent_bytes, &spent, &identity, &lease)?;

    match recover_pending_adoption_until(
        &checkout_dir,
        &challenge_dir,
        &identity,
        &spent.token_digest,
        deadline,
    )? {
        PendingRecovery::None | PendingRecovery::Committed => {}
        PendingRecovery::RolledBack | PendingRecovery::Staged | PendingRecovery::Revoked => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "active adoption changed while revocation was prepared",
            ));
        }
    }

    ensure_deadline(deadline)?;
    let revoked_path = checkout_dir.join(format!(".revoked-{receipt_id}.json"));
    fs::rename(&lease_path, &revoked_path).context("failed to revoke adopted checkout lease")?;
    sync_directory(&checkout_dir).context("revoked lease tombstone could not be made durable")?;
    if !recover_revocation_tombstone(
        &checkout_dir,
        &challenge_dir,
        &receipts_dir,
        &identity,
        receipt_id,
        deadline,
    )? {
        return Err(domain_error(
            DirtyCheckoutErrorKind::RevocationFailed,
            "durable revocation tombstone could not be recovered",
        ));
    }
    Ok(())
}

fn validate_receipt_matches_lease(receipt: &ReceiptRecord, lease: &LeaseRecord) -> Result<()> {
    let adoption = lease.adoption().ok_or_else(|| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "checkout lease has no dirty adoption",
        )
    })?;
    if lease.schema() != LEASE_V2_SCHEMA
        || lease.session_key() != receipt.session_key
        || lease.checkout_instance() != receipt.checkout_instance
        || adoption.schema != ADOPTION_SCHEMA
        || adoption.receipt_schema != receipt.schema
        || adoption.receipt_id != receipt.receipt_id
        || adoption.snapshot_id != receipt.snapshot_id
        || adoption.authorization_turn_digest != receipt.authorization_turn_digest
        || adoption.reason_digest != receipt.reason_digest
        || adoption.adopted_at != receipt.adopted_at
        || adoption.challenge_digest != receipt.challenge_digest
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "receipt does not match the dirty-checkout adoption",
        ));
    }
    Ok(())
}

fn validate_spent_challenge_matches_lease(
    bytes: &[u8],
    spent: &ChallengeRecord,
    identity: &CheckoutIdentity,
    lease: &LeaseRecord,
) -> Result<()> {
    validate_challenge_identity(spent, identity, &spent.token_digest)?;
    let adoption = lease.adoption().ok_or_else(|| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "checkout lease has no dirty adoption",
        )
    })?;
    if lease.schema() != LEASE_V2_SCHEMA
        || sha256_hex(bytes) != adoption.challenge_digest
        || spent.session_key != lease.session_key()
        || spent.snapshot_id != adoption.snapshot_id
        || spent.authorization_turn_digest != adoption.authorization_turn_digest
        || spent.issued_at != adoption.challenge_issued_at
        || adoption.adopted_at < spent.issued_at
        || adoption.adopted_at >= spent.expires_at
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "spent challenge does not match the dirty-checkout adoption",
        ));
    }
    Ok(())
}

fn recover_revocation_tombstone(
    checkout_dir: &Path,
    challenge_dir: &Path,
    receipts_dir: &Path,
    identity: &CheckoutIdentity,
    receipt_id: &str,
    deadline: Instant,
) -> Result<bool> {
    ensure_deadline(deadline)?;
    let tombstone_path = checkout_dir.join(format!(".revoked-{receipt_id}.json"));
    let Some(tombstone_bytes) = read_optional_private(&tombstone_path, "revoked adoption")? else {
        return Ok(false);
    };
    let tombstone = parse_lease(&tombstone_bytes)?;
    validate_lease(&tombstone, identity)?;
    let adoption = tombstone.adoption().ok_or_else(|| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "revoked adoption tombstone has no adoption identity",
        )
    })?;
    if adoption.receipt_id != receipt_id {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "revoked adoption tombstone receipt identity is inconsistent",
        ));
    }

    let mut budget = RecoveryScanBudget::new(deadline);
    let mut token_digests = Vec::new();
    for entry in fs::read_dir(checkout_dir).context("revocation recovery state is unavailable")? {
        let entry = entry.context("revocation recovery state is unreadable")?;
        let name = entry.file_name();
        let token_digest = name
            .to_str()
            .and_then(|name| name.strip_prefix(".pending-adoption-"))
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|digest| is_lower_hex(digest, 64));
        budget.charge(&entry, token_digest.is_some())?;
        if let Some(token_digest) = token_digest {
            token_digests.push(token_digest.to_string());
        }
    }
    budget.check_deadline()?;

    for token_digest in token_digests {
        budget.check_deadline()?;
        let Some(pending_bytes) = read_optional_private_with_limit(
            &pending_adoption_path(checkout_dir, &token_digest),
            "pending adoption",
            MAX_PENDING_STATE_FILE_BYTES,
        )?
        else {
            continue;
        };
        let pending: PendingAdoptionRecord =
            serde_json::from_slice(&pending_bytes).map_err(|_| {
                domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "pending dirty-checkout adoption state is malformed",
                )
            })?;
        validate_pending_adoption(&pending, identity, &token_digest)?;
        if pending.receipt_id == receipt_id {
            if pending.challenge_digest != adoption.challenge_digest
                || recover_pending_adoption_until(
                    checkout_dir,
                    challenge_dir,
                    identity,
                    &token_digest,
                    deadline,
                )? != PendingRecovery::Revoked
            {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "revoked adoption pending state is inconsistent",
                ));
            }
            break;
        }
    }
    budget.check_deadline()?;

    let receipt_path = receipts_dir.join(format!("{receipt_id}.json"));
    if let Some(bytes) = read_optional_private(&receipt_path, "revoked adoption receipt")? {
        let receipt: ReceiptRecord = serde_json::from_slice(&bytes).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revoked adoption receipt is malformed",
            )
        })?;
        validate_receipt(&receipt, identity, receipt_id)?;
        validate_receipt_matches_lease(&receipt, &tombstone)?;
    }
    let spent_path = receipts_dir.join(format!(".challenge-{receipt_id}.json"));
    let mut token_digest = None;
    if let Some(bytes) = read_optional_private(&spent_path, "revoked adoption challenge")? {
        let spent: ChallengeRecord = serde_json::from_slice(&bytes).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revoked adoption challenge is malformed",
            )
        })?;
        validate_spent_challenge_matches_lease(&bytes, &spent, identity, &tombstone)?;
        token_digest = Some(spent.token_digest);
    }
    budget.check_deadline()?;
    for path in [&receipt_path, &spent_path] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("revoked adoption artifact cleanup failed"),
        }
    }
    if let Some(token_digest) = token_digest {
        let challenge_path = challenge_dir.join(format!("{token_digest}.json"));
        match fs::remove_file(&challenge_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("revoked adoption challenge cleanup failed"),
        }
    }
    sync_directory(receipts_dir)
        .context("revoked adoption receipt cleanup could not be made durable")?;
    sync_directory(challenge_dir)
        .context("revoked adoption challenge cleanup could not be made durable")?;
    sync_directory(checkout_dir)
        .context("revoked adoption tombstone could not be confirmed durable")?;
    prune_revocation_tombstones(checkout_dir, identity, receipt_id, deadline)?;
    Ok(true)
}

fn prune_revocation_tombstones(
    checkout_dir: &Path,
    identity: &CheckoutIdentity,
    current_receipt_id: &str,
    deadline: Instant,
) -> Result<()> {
    let mut protected = HashSet::from([current_receipt_id.to_string()]);
    let mut candidates = HashSet::new();
    let mut budget = RecoveryScanBudget::new(deadline);
    for entry in fs::read_dir(checkout_dir).context("revocation retention state is unavailable")? {
        let entry = entry.context("revocation retention state entry is unreadable")?;
        let name = entry.file_name();
        let token_digest = name
            .to_str()
            .and_then(|name| name.strip_prefix(".pending-adoption-"))
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|digest| is_lower_hex(digest, 64));
        let tombstone_receipt = name
            .to_str()
            .and_then(|name| name.strip_prefix(".revoked-"))
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|receipt_id| is_lower_hex(receipt_id, 64));
        budget.charge(
            &entry,
            token_digest.is_some() || tombstone_receipt.is_some(),
        )?;
        if let Some(receipt_id) = tombstone_receipt {
            candidates.insert(receipt_id.to_string());
        }
        let Some(token_digest) = token_digest else {
            continue;
        };
        let bytes = read_private_regular(&entry.path(), MAX_PENDING_STATE_FILE_BYTES, true)?;
        let pending: PendingAdoptionRecord = serde_json::from_slice(&bytes).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "pending adoption tombstone reference is malformed",
            )
        })?;
        validate_pending_adoption(&pending, identity, token_digest)?;
        candidates.insert(pending.receipt_id.clone());
        protected.insert(pending.receipt_id);
    }
    budget.check_deadline()?;
    prune_revocation_tombstone_candidates(checkout_dir, identity, &protected, candidates, deadline)
}

fn prune_revocation_tombstone_candidates<I>(
    checkout_dir: &Path,
    identity: &CheckoutIdentity,
    protected: &HashSet<String>,
    candidates: I,
    deadline: Instant,
) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let mut tombstones = Vec::new();
    for receipt_id in candidates {
        ensure_deadline(deadline)?;
        let path = checkout_dir.join(format!(".revoked-{receipt_id}.json"));
        let Some(bytes) = read_optional_private(&path, "revocation tombstone")? else {
            continue;
        };
        let lease = parse_lease(&bytes)?;
        validate_lease(&lease, identity)?;
        let adoption = lease.adoption().ok_or_else(|| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revocation tombstone has no adoption identity",
            )
        })?;
        if adoption.receipt_id != receipt_id {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "revocation tombstone filename does not match its adoption identity",
            ));
        }
        tombstones.push((
            protected.contains(&receipt_id),
            adoption.adopted_at,
            receipt_id,
        ));
    }
    tombstones.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let remove_count = tombstones.len().saturating_sub(MAX_REVOCATION_TOMBSTONES);
    let mut removed = false;
    for (_, _, receipt_id) in tombstones
        .into_iter()
        .filter(|(protected, _, _)| !protected)
        .take(remove_count)
    {
        ensure_deadline(deadline)?;
        fs::remove_file(checkout_dir.join(format!(".revoked-{receipt_id}.json")))
            .context("expired revocation tombstone cleanup failed")?;
        removed = true;
    }
    if removed {
        sync_directory(checkout_dir)
            .context("revocation tombstone retention could not be made durable")?;
    }
    ensure_deadline(deadline)
}

fn snapshot_once(checkout: &Path, deadline: Instant) -> Result<SnapshotPass> {
    ensure_deadline(deadline)?;
    let identity = resolve_checkout(checkout, true, deadline)?;
    let mut budget = SnapshotBudget::default();
    reject_command_bearing_git_config(&mut budget, &identity.root, deadline)?;
    reject_active_git_operation(&identity)?;

    let branch_output = snapshot_git_output(
        &mut budget,
        &identity.root,
        &["symbolic-ref", "--quiet", "HEAD"],
        4096,
        deadline,
    )?;
    let branch_raw = if branch_output.status.success() {
        strip_git_line(&branch_output.stdout)?.to_vec()
    } else if branch_output.status.code() == Some(1) {
        Vec::new()
    } else {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git branch identity is unavailable",
        ));
    };

    let head_output = snapshot_git_output(
        &mut budget,
        &identity.root,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        256,
        deadline,
    )?;
    let head_oid = if head_output.status.success() {
        let head_raw = strip_git_line(&head_output.stdout)?;
        if !(40..=64).contains(&head_raw.len()) || !head_raw.iter().all(u8::is_ascii_hexdigit) {
            return Err(domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "Git HEAD identity is malformed",
            ));
        }
        String::from_utf8(head_raw.to_vec()).map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "Git HEAD identity is malformed",
            )
        })?
    } else if !branch_raw.is_empty() {
        validate_unborn_head(&mut budget, &identity.root, &branch_raw, deadline)?
    } else {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git HEAD identity is unavailable",
        ));
    };

    let index_output = snapshot_git_output(
        &mut budget,
        &identity.root,
        &["ls-files", "--stage", "-z"],
        MAX_GIT_OUTPUT_BYTES,
        deadline,
    )?;
    if !index_output.status.success() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git index listing failed",
        ));
    }
    let mut tracked = parse_index_bounded(&index_output.stdout, MAX_ENTRY_COUNT, &mut budget)?;
    drop(index_output);

    let flags_output = snapshot_git_output(
        &mut budget,
        &identity.root,
        &["ls-files", "-v", "-z"],
        MAX_GIT_OUTPUT_BYTES,
        deadline,
    )?;
    if !flags_output.status.success() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git index flags are unavailable",
        ));
    }
    validate_index_flags(&flags_output.stdout, &tracked, &mut budget)?;
    drop(flags_output);

    reject_initialized_submodule_config(&identity.root, &tracked, &mut budget, deadline)?;

    let staged_output = snapshot_git_output(
        &mut budget,
        &identity.root,
        &[
            "diff",
            "--cached",
            "--quiet",
            "--no-ext-diff",
            "--ignore-submodules=none",
            "--",
        ],
        1,
        deadline,
    )?;
    let staged_dirty = match staged_output.status.code() {
        Some(0) => false,
        Some(1) => true,
        _ => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "Git staged-state probe failed",
            ));
        }
    };
    drop(staged_output);

    let unstaged_output = snapshot_git_output(
        &mut budget,
        &identity.root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-ext-diff",
            "--ignore-submodules=none",
            "--",
        ],
        MAX_GIT_OUTPUT_BYTES,
        deadline,
    )?;
    if !unstaged_output.status.success() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git unstaged-path listing failed",
        ));
    }
    let mut unstaged =
        parse_nul_paths_bounded(&unstaged_output.stdout, MAX_ENTRY_COUNT, &mut budget)?;
    drop(unstaged_output);

    let untracked_output = snapshot_git_output(
        &mut budget,
        &identity.root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        MAX_GIT_OUTPUT_BYTES,
        deadline,
    )?;
    if !untracked_output.status.success() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git untracked listing failed",
        ));
    }
    let remaining_entries = MAX_ENTRY_COUNT.saturating_sub(tracked.len());
    let mut untracked =
        parse_nul_paths_bounded(&untracked_output.stdout, remaining_entries, &mut budget)?;
    drop(untracked_output);

    if !staged_dirty && unstaged.is_empty() && untracked.is_empty() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::CleanCheckout,
            "checkout is clean",
        ));
    }

    tracked.sort_by(|left, right| left.path.cmp(&right.path));
    unstaged.sort();
    untracked.sort();
    ensure_unique_paths(tracked.iter().map(|entry| entry.path.as_slice()))?;
    ensure_unique_paths(unstaged.iter().map(Vec::as_slice))?;
    ensure_unique_paths(untracked.iter().map(Vec::as_slice))?;
    let submodule_paths: HashSet<&[u8]> = tracked
        .iter()
        .filter(|entry| entry.mode.as_slice() == b"160000")
        .map(|entry| entry.path.as_slice())
        .collect();
    reject_special_filesystem_objects(&identity.root, &submodule_paths, &mut budget, deadline)
        .map_err(|error| {
            command_error(
                error,
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "checkout contains unsupported filesystem state",
            )
        })?;
    let tracked_paths: HashSet<&[u8]> = tracked.iter().map(|entry| entry.path.as_slice()).collect();
    if !unstaged
        .iter()
        .all(|path| tracked_paths.contains(path.as_slice()))
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git unstaged-path listing does not match the index",
        ));
    }
    let unstaged_count = unstaged.len();
    let unstaged_paths: HashSet<Vec<u8>> = unstaged.into_iter().collect();

    let mut hasher = FramedHasher::new();
    hasher.field(b"schema", SNAPSHOT_SCHEMA.as_bytes());
    hasher.field(b"repository_key", identity.repository_key.as_bytes());
    hasher.field(b"checkout_key", identity.checkout_key.as_bytes());
    hasher.field(b"checkout_instance", identity.checkout_instance.as_bytes());
    hasher.field(b"head_oid", head_oid.as_bytes());
    hasher.field(b"branch_ref", &branch_raw);

    for entry in &tracked {
        ensure_deadline(deadline)?;
        hasher.field(b"index_mode", &entry.mode);
        hasher.field(b"index_oid", &entry.oid);
        hasher.field(b"index_path", &entry.path);
        let is_submodule = entry.mode.as_slice() == b"160000";
        if is_submodule || unstaged_paths.contains(entry.path.as_slice()) {
            if !is_submodule {
                hasher.field(b"unstaged_path", &entry.path);
            }
            hash_worktree_object(
                &mut hasher,
                &identity.root,
                &entry.path,
                is_submodule,
                Some(&entry.oid),
                &mut budget,
                deadline,
            )?;
        }
    }
    for path in &untracked {
        ensure_deadline(deadline)?;
        hasher.field(b"untracked_path", path);
        hash_worktree_object(
            &mut hasher,
            &identity.root,
            path,
            false,
            None,
            &mut budget,
            deadline,
        )?;
    }
    hasher.field(b"tracked_count", &(tracked.len() as u64).to_be_bytes());
    hasher.field(b"unstaged_count", &(unstaged_count as u64).to_be_bytes());
    hasher.field(b"untracked_count", &(untracked.len() as u64).to_be_bytes());
    hasher.field(b"hashed_bytes", &budget.file_bytes.to_be_bytes());

    Ok(SnapshotPass {
        snapshot: DirtySnapshot {
            schema: SNAPSHOT_SCHEMA,
            repository_key: identity.repository_key,
            checkout_key: identity.checkout_key,
            checkout_instance: identity.checkout_instance,
            snapshot_id: hasher.finish(),
            head_oid,
            branch_ref_digest: sha256_hex(&branch_raw),
            tracked_entries: tracked.len(),
            untracked_entries: untracked.len(),
            hashed_bytes: budget.file_bytes,
        },
    })
}

fn reject_command_bearing_git_config(
    budget: &mut SnapshotBudget,
    cwd: &Path,
    deadline: Instant,
) -> Result<()> {
    const COMMAND_BEARING_CONFIG: &str = r"^(include\.path|includeif\..*\.path|filter\..*\.(clean|smudge|process)|diff\..*\.(command|textconv))$";
    let output = snapshot_git_output(
        budget,
        cwd,
        &[
            "config",
            "--no-includes",
            "--name-only",
            "--null",
            "--get-regexp",
            COMMAND_BEARING_CONFIG,
        ],
        1024 * 1024,
        deadline,
    )?;
    match output.status.code() {
        Some(1) if output.stdout.is_empty() => Ok(()),
        Some(0) => Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "repository command-bearing Git configuration is unsupported",
        )),
        _ => Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "repository Git configuration could not be validated",
        )),
    }
}

fn validate_git_toplevel(
    budget: &mut SnapshotBudget,
    expected: &Path,
    deadline: Instant,
) -> Result<()> {
    let output = snapshot_git_output(
        budget,
        expected,
        &["rev-parse", "--show-toplevel"],
        1024 * 1024,
        deadline,
    )?;
    ensure!(
        output.status.success(),
        "submodule Git worktree identity is unavailable"
    );
    let resolved = canonical_git_path(expected, strip_git_line(&output.stdout)?)?;
    ensure!(
        resolved == expected,
        "submodule Git worktree identity does not match its index path"
    );
    Ok(())
}

fn reject_initialized_submodule_config(
    root: &Path,
    tracked: &[IndexEntry],
    budget: &mut SnapshotBudget,
    deadline: Instant,
) -> Result<()> {
    let mut visited = HashSet::new();
    reject_initialized_submodule_config_inner(root, tracked, budget, deadline, &mut visited)
}

fn reject_initialized_submodule_config_inner(
    root: &Path,
    tracked: &[IndexEntry],
    budget: &mut SnapshotBudget,
    deadline: Instant,
    visited: &mut HashSet<PathBuf>,
) -> Result<()> {
    for entry in tracked
        .iter()
        .filter(|entry| entry.mode.as_slice() == b"160000")
    {
        ensure_deadline(deadline)?;
        let path = root.join(OsStr::from_bytes(&entry.path));
        let result = (|| -> Result<()> {
            verify_parent_within_root(root, &path)?;
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    return Err(error).context("submodule metadata is unavailable");
                }
            };
            ensure!(
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                "submodule checkout path is not a directory"
            );
            let canonical =
                fs::canonicalize(&path).context("submodule path could not be verified")?;
            ensure!(
                canonical.starts_with(root),
                "submodule path escapes the checkout"
            );
            ensure!(
                visited.insert(canonical.clone()),
                "submodule checkout graph contains a cycle"
            );
            validate_git_toplevel(budget, &canonical, deadline)?;
            reject_command_bearing_git_config(budget, &canonical, deadline)?;
            let index_output = snapshot_git_output(
                budget,
                &canonical,
                &["ls-files", "--stage", "-z"],
                MAX_GIT_OUTPUT_BYTES,
                deadline,
            )?;
            ensure!(
                index_output.status.success(),
                "submodule index listing failed"
            );
            let nested = parse_index_bounded(&index_output.stdout, MAX_ENTRY_COUNT, budget)?;
            drop(index_output);
            let flags_output = snapshot_git_output(
                budget,
                &canonical,
                &["ls-files", "-v", "-z"],
                MAX_GIT_OUTPUT_BYTES,
                deadline,
            )?;
            ensure!(
                flags_output.status.success(),
                "submodule index flags are unavailable"
            );
            validate_index_flags(&flags_output.stdout, &nested, budget)?;
            reject_initialized_submodule_config_inner(
                &canonical, &nested, budget, deadline, visited,
            )
        })();
        if let Err(error) = result {
            return Err(command_error(
                error,
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "submodule Git configuration is unsupported",
            ));
        }
    }
    Ok(())
}

fn snapshot_git_output(
    budget: &mut SnapshotBudget,
    cwd: &Path,
    args: &[&str],
    command_limit: usize,
    deadline: Instant,
) -> Result<std::process::Output> {
    let limit = budget.output_limit(command_limit)?;
    let output = git_output(cwd, args, limit, deadline)?;
    budget.charge_output(&output)?;
    Ok(output)
}

fn validate_unborn_head(
    budget: &mut SnapshotBudget,
    cwd: &Path,
    branch_ref: &[u8],
    deadline: Instant,
) -> Result<String> {
    if !branch_ref.starts_with(b"refs/heads/") {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "symbolic HEAD does not identify a local branch",
        ));
    }
    let args = [
        OsString::from("show-ref"),
        OsString::from("--verify"),
        OsString::from("--quiet"),
        OsString::from_vec(branch_ref.to_vec()),
    ];
    let limit = budget.output_limit(1024)?;
    let output = git_output_os(cwd, &args, limit, deadline)?;
    budget.charge_output(&output)?;
    if output.status.code() == Some(1) {
        return Ok(unborn_head_identity(branch_ref));
    }
    Err(domain_error(
        DirtyCheckoutErrorKind::UnsupportedGitState,
        "symbolic HEAD is dangling, inaccessible, or does not identify a commit",
    ))
}

fn unborn_head_identity(branch_ref: &[u8]) -> String {
    let mut hasher = FramedHasher::new();
    hasher.field(b"domain", b"agent-runtime.git-head.unborn.v1");
    hasher.field(b"symbolic_ref", branch_ref);
    format!("{UNBORN_HEAD_PREFIX}{}", hasher.finish())
}

fn resolve_checkout(
    checkout: &Path,
    create_instance: bool,
    deadline: Instant,
) -> Result<CheckoutIdentity> {
    let requested =
        fs::canonicalize(checkout).context("checkout identity could not be resolved")?;
    let requested_root = requested_checkout_root(&requested)?;
    let root_output = git_output_os(
        &requested,
        &[
            OsString::from("rev-parse"),
            OsString::from("--show-toplevel"),
        ],
        1024 * 1024,
        deadline,
    )?;
    ensure!(root_output.status.success(), "path is not a Git checkout");
    let root = canonical_git_path(&requested, strip_git_line(&root_output.stdout)?)?;
    if root != requested_root {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git checkout top-level does not match the requested physical checkout",
        ));
    }
    let git_dir_output = git_output(
        &root,
        &["rev-parse", "--absolute-git-dir"],
        1024 * 1024,
        deadline,
    )?;
    let common_dir_output = git_output(
        &root,
        &["rev-parse", "--git-common-dir"],
        1024 * 1024,
        deadline,
    )?;
    ensure!(
        git_dir_output.status.success() && common_dir_output.status.success(),
        "Git checkout identity could not be resolved"
    );
    let git_dir = canonical_git_path(&root, strip_git_line(&git_dir_output.stdout)?)?;
    let common_dir = canonical_git_path(&root, strip_git_line(&common_dir_output.stdout)?)?;
    let repository_key = sha256_hex(common_dir.as_os_str().as_bytes());
    let checkout_key = sha256_hex(root.as_os_str().as_bytes());
    let checkout_instance = read_checkout_instance(&git_dir, create_instance)?;
    Ok(CheckoutIdentity {
        root,
        git_dir,
        common_dir,
        repository_key,
        checkout_key,
        checkout_instance,
    })
}

fn requested_checkout_root(requested: &Path) -> Result<PathBuf> {
    for ancestor in requested.ancestors() {
        let marker = ancestor.join(".git");
        match fs::symlink_metadata(&marker) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::UnsupportedGitState,
                    "requested checkout has a symbolic Git identity marker",
                ));
            }
            Ok(metadata) if metadata.is_file() || metadata.is_dir() => {
                return fs::canonicalize(ancestor)
                    .context("requested checkout root could not be canonicalized");
            }
            Ok(_) => {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::UnsupportedGitState,
                    "requested checkout has an unsupported Git identity marker",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("requested checkout Git identity is unavailable");
            }
        }
    }
    Err(domain_error(
        DirtyCheckoutErrorKind::UnsupportedGitState,
        "requested path is not inside a physical Git checkout",
    ))
}

fn canonical_git_path(base: &Path, raw: &[u8]) -> Result<PathBuf> {
    ensure!(
        !raw.is_empty() && !raw.contains(&0),
        "Git path identity is malformed"
    );
    let path = PathBuf::from(OsString::from_vec(raw.to_vec()));
    let absolute = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    fs::canonicalize(absolute).context("Git path identity could not be canonicalized")
}

fn read_checkout_instance(git_dir: &Path, create: bool) -> Result<String> {
    let path = git_dir.join(INSTANCE_FILE);
    match read_private_regular(&path, 128, false) {
        Ok(raw) if !raw.is_empty() => return parse_instance(&raw),
        Ok(_) => {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "checkout instance sentinel is empty",
            ));
        }
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|io| io.kind() == io::ErrorKind::NotFound) => {}
        Err(error) => {
            return Err(command_error(
                error,
                DirtyCheckoutErrorKind::MalformedState,
                "checkout instance sentinel is untrusted",
            ));
        }
    }
    if !create {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "checkout instance sentinel is missing",
        ));
    }
    let value = random_hex(16)?;
    atomic_create_once(&path, format!("{value}\n").as_bytes())?;
    let raw = read_private_regular(&path, 128, true)?;
    parse_instance(&raw)
}

fn parse_instance(raw: &[u8]) -> Result<String> {
    let value = std::str::from_utf8(raw)
        .map_err(|_| {
            domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "checkout instance sentinel is malformed",
            )
        })?
        .trim();
    if !is_lower_hex(value, 32) {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "checkout instance sentinel is malformed",
        ));
    }
    Ok(value.to_string())
}

fn reject_active_git_operation(identity: &CheckoutIdentity) -> Result<()> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "BISECT_LOG",
        "index.lock",
    ] {
        match fs::symlink_metadata(identity.git_dir.join(marker)) {
            Ok(_) => {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::UnsupportedGitState,
                    "checkout has an active or ambiguous Git operation",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::ResourceUnavailable,
                    format!("Git operation state could not be verified: {error}"),
                ));
            }
        }
    }
    Ok(())
}

fn parse_index_bounded(
    raw: &[u8],
    entry_limit: usize,
    budget: &mut SnapshotBudget,
) -> Result<Vec<IndexEntry>> {
    let payload = nul_payload(raw)?;
    if payload.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for record in payload.split(|byte| *byte == 0) {
        if entries.len() >= entry_limit {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot tracked entry count exceeds the supported limit",
            ));
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("Git index output is malformed")?;
        let metadata = &record[..tab];
        let path = &record[tab + 1..];
        let mut fields = metadata.split(|byte| *byte == b' ');
        let mode = fields.next().context("Git index output is malformed")?;
        let oid = fields.next().context("Git index output is malformed")?;
        let stage = fields.next().context("Git index output is malformed")?;
        ensure!(fields.next().is_none(), "Git index output is malformed");
        ensure!(
            mode.len() == 6 && mode.iter().all(u8::is_ascii_digit),
            "Git index mode is malformed"
        );
        ensure!(
            (40..=64).contains(&oid.len()) && oid.iter().all(u8::is_ascii_hexdigit),
            "Git index object identity is malformed"
        );
        if oid.iter().all(|byte| *byte == b'0') {
            return Err(domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "intent-to-add index entries are unsupported",
            ));
        }
        if stage != b"0" {
            return Err(domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "unmerged index stages are unsupported",
            ));
        }
        validate_repo_relative_path(path)?;
        budget.charge_path(path.len())?;
        entries.push(IndexEntry {
            mode: mode.to_vec(),
            oid: oid.to_vec(),
            path: path.to_vec(),
        });
    }
    Ok(entries)
}

fn parse_nul_paths_bounded(
    raw: &[u8],
    entry_limit: usize,
    budget: &mut SnapshotBudget,
) -> Result<Vec<Vec<u8>>> {
    let payload = nul_payload(raw)?;
    if payload.is_empty() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for path in payload.split(|byte| *byte == 0) {
        if paths.len() >= entry_limit {
            return Err(domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                "dirty snapshot path entry count exceeds the supported limit",
            ));
        }
        validate_repo_relative_path(path)?;
        budget.charge_path(path.len())?;
        paths.push(path.to_vec());
    }
    Ok(paths)
}

#[cfg(test)]
fn parse_nul_paths(raw: &[u8]) -> Result<Vec<Vec<u8>>> {
    parse_nul_paths_bounded(raw, MAX_ENTRY_COUNT, &mut SnapshotBudget::default())
}

fn validate_index_flags(
    raw: &[u8],
    tracked: &[IndexEntry],
    budget: &mut SnapshotBudget,
) -> Result<()> {
    let payload = nul_payload(raw)?;
    if payload.is_empty() {
        if tracked.is_empty() {
            return Ok(());
        }
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git index flag listing does not match the index",
        ));
    }
    let mut count = 0_usize;
    for record in payload.split(|byte| *byte == 0) {
        if count >= tracked.len() || record.len() < 3 || record[1] != b' ' {
            return Err(domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "Git index flag listing does not match the index",
            ));
        }
        let path = &record[2..];
        validate_repo_relative_path(path)?;
        budget.charge_path(path.len())?;
        if record[0] != b'H' {
            return Err(domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "tracked index flags that can hide worktree drift are unsupported",
            ));
        }
        if path != tracked[count].path.as_slice() {
            return Err(domain_error(
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "Git index flag listing does not match the index",
            ));
        }
        count += 1;
    }
    if count != tracked.len() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::UnsupportedGitState,
            "Git index flag listing does not match the index",
        ));
    }
    Ok(())
}

fn nul_payload(raw: &[u8]) -> Result<&[u8]> {
    if raw.is_empty() {
        return Ok(raw);
    }
    ensure!(
        raw.last() == Some(&0),
        "NUL-delimited Git output is malformed"
    );
    let payload = &raw[..raw.len() - 1];
    ensure!(
        !payload.is_empty()
            && !payload
                .split(|byte| *byte == 0)
                .any(|record| record.is_empty()),
        "NUL-delimited Git output is malformed"
    );
    Ok(payload)
}

fn ensure_unique_paths<'a>(paths: impl Iterator<Item = &'a [u8]>) -> Result<()> {
    let mut seen = HashSet::new();
    for path in paths {
        ensure!(
            seen.insert(path.to_vec()),
            "Git path listing contains duplicate entries"
        );
    }
    Ok(())
}

fn validate_repo_relative_path(raw: &[u8]) -> Result<()> {
    ensure!(
        !raw.is_empty() && !raw.contains(&0),
        "Git path is malformed"
    );
    let path = Path::new(OsStr::from_bytes(raw));
    ensure!(!path.is_absolute(), "absolute Git paths are unsupported");
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "Git path escapes the checkout"
    );
    Ok(())
}

fn reject_special_filesystem_objects(
    root: &Path,
    submodule_paths: &HashSet<&[u8]>,
    budget: &mut SnapshotBudget,
    deadline: Instant,
) -> Result<()> {
    let mut directories = vec![PathBuf::new()];
    while let Some(relative_directory) = directories.pop() {
        ensure_deadline(deadline)?;
        let directory = root.join(&relative_directory);
        let metadata = fs::symlink_metadata(&directory)
            .context("checkout directory metadata is unavailable")?;
        ensure!(
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
            "checkout directory changed during snapshot"
        );
        let canonical = fs::canonicalize(&directory)
            .context("checkout directory identity could not be verified")?;
        ensure!(
            canonical.starts_with(root),
            "checkout directory escapes the checkout"
        );
        let entries =
            fs::read_dir(&directory).context("checkout directory could not be scanned")?;
        for entry in entries {
            ensure_deadline(deadline)?;
            let entry = entry.context("checkout directory entry is unavailable")?;
            if relative_directory.as_os_str().is_empty() && entry.file_name().as_bytes() == b".git"
            {
                continue;
            }
            budget.charge_traversal_entry()?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .context("checkout directory entry escapes the checkout")?;
            let relative_bytes = relative.as_os_str().as_bytes();
            let metadata = fs::symlink_metadata(&path)
                .context("checkout directory entry metadata is unavailable")?;
            if submodule_paths.contains(relative_bytes) {
                ensure!(
                    metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                    "submodule checkout path is not a directory"
                );
                continue;
            }
            if metadata.file_type().is_dir() {
                budget.charge_path(relative_bytes.len())?;
                directories.push(relative.to_path_buf());
            } else if metadata.file_type().is_symlink() {
                let target = fs::read_link(&path).context("symlink target could not be read")?;
                if target.as_os_str().as_bytes().len() > 64 * 1024 {
                    return Err(domain_error(
                        DirtyCheckoutErrorKind::ResourceUnavailable,
                        "symlink target exceeds the supported limit",
                    ));
                }
                ensure_symlink_stays_within_root(
                    root,
                    path.parent().expect("directory entry parent"),
                    &target,
                )?;
                let after = fs::symlink_metadata(&path)
                    .context("symlink changed during checkout traversal")?;
                ensure!(
                    same_metadata(&metadata, &after),
                    "symlink changed during checkout traversal"
                );
            } else if metadata.file_type().is_file() {
                ensure!(
                    metadata.nlink() == 1,
                    "multiply-linked files are unsupported"
                );
            } else {
                bail!("special filesystem objects are unsupported");
            }
        }
    }
    Ok(())
}

fn hash_worktree_object(
    hasher: &mut FramedHasher,
    root: &Path,
    raw_path: &[u8],
    submodule: bool,
    index_oid: Option<&Vec<u8>>,
    budget: &mut SnapshotBudget,
    deadline: Instant,
) -> Result<()> {
    let relative = PathBuf::from(OsString::from_vec(raw_path.to_vec()));
    let path = root.join(&relative);
    verify_parent_within_root(root, &path)?;
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if submodule {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::UnsupportedGitState,
                    "dirty or unavailable submodules are unsupported",
                ));
            }
            hasher.field(b"worktree_kind", b"missing");
            return Ok(());
        }
        Err(error) => return Err(error).context("worktree object metadata is unavailable"),
    };

    if submodule {
        let result = (|| -> Result<()> {
            ensure!(
                before.file_type().is_dir(),
                "dirty or unavailable submodules are unsupported"
            );
            let canonical =
                fs::canonicalize(&path).context("submodule path could not be verified")?;
            ensure!(
                canonical.starts_with(root),
                "submodule path escapes the checkout"
            );
            validate_git_toplevel(budget, &canonical, deadline)?;
            reject_command_bearing_git_config(budget, &canonical, deadline)?;
            reject_special_filesystem_objects(&canonical, &HashSet::new(), budget, deadline)?;
            let head = snapshot_git_output(
                budget,
                &canonical,
                &["rev-parse", "--verify", "HEAD^{commit}"],
                256,
                deadline,
            )?;
            ensure!(
                head.status.success(),
                "dirty or unavailable submodules are unsupported"
            );
            let submodule_head = strip_git_line(&head.stdout)?;
            ensure!(
                index_oid.is_some_and(|oid| oid.as_slice() == submodule_head),
                "dirty or unavailable submodules are unsupported"
            );
            let status = snapshot_git_output(
                budget,
                &canonical,
                &[
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--untracked-files=all",
                    "--ignore-submodules=none",
                ],
                MAX_GIT_OUTPUT_BYTES,
                deadline,
            )?;
            ensure!(
                status.status.success() && status.stdout.is_empty(),
                "dirty submodules are unsupported"
            );
            hasher.field(b"worktree_kind", b"submodule");
            hasher.field(b"worktree_digest", submodule_head);
            Ok(())
        })();
        return result.map_err(|error| {
            command_error(
                error,
                DirtyCheckoutErrorKind::UnsupportedGitState,
                "dirty or unavailable submodules are unsupported",
            )
        });
    }

    if before.file_type().is_symlink() {
        let target = fs::read_link(&path).context("symlink target could not be read")?;
        ensure_symlink_stays_within_root(root, path.parent().expect("path parent"), &target)?;
        let after = fs::symlink_metadata(&path).context("symlink changed during snapshot")?;
        ensure!(
            same_metadata(&before, &after),
            "symlink changed during snapshot"
        );
        let target_bytes = target.as_os_str().as_bytes();
        ensure!(
            target_bytes.len() <= 64 * 1024,
            "symlink target exceeds the supported limit"
        );
        hasher.field(b"worktree_kind", b"symlink");
        hasher.field(b"worktree_digest", &Sha256::digest(target_bytes));
        return Ok(());
    }
    ensure!(
        before.file_type().is_file(),
        "special filesystem objects are unsupported"
    );
    ensure!(before.nlink() == 1, "multiply-linked files are unsupported");
    if before.len() > MAX_FILE_BYTES {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "file exceeds the dirty snapshot size limit",
        ));
    }
    let remaining_total = MAX_TOTAL_BYTES.saturating_sub(budget.file_bytes);
    if before.len() > remaining_total {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty snapshot total bytes exceed the supported limit",
        ));
    }
    let canonical = fs::canonicalize(&path).context("worktree file could not be verified")?;
    ensure!(
        canonical.starts_with(root),
        "worktree file escapes the checkout"
    );

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)
        .context("worktree file could not be opened safely")?;
    let opened = file
        .metadata()
        .context("worktree file metadata is unavailable")?;
    ensure!(
        same_metadata(&before, &opened),
        "worktree file changed before hashing"
    );
    let mut digest = Sha256::new();
    let mut remaining = before.len();
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        ensure_deadline(deadline)?;
        let read_limit =
            usize::try_from(remaining.min(buffer.len() as u64)).expect("bounded read size");
        let read = file
            .read(&mut buffer[..read_limit])
            .context("worktree file read failed")?;
        ensure!(read > 0, "worktree file changed while hashing");
        digest.update(&buffer[..read]);
        remaining -= read as u64;
    }
    let mut extra = [0_u8; 1];
    ensure!(
        file.read(&mut extra)
            .context("worktree file final read failed")?
            == 0,
        "worktree file grew while hashing"
    );
    let after = file
        .metadata()
        .context("worktree file final metadata is unavailable")?;
    ensure!(
        same_metadata(&opened, &after),
        "worktree file changed while hashing"
    );
    let pathname_after =
        fs::symlink_metadata(&path).context("worktree file pathname changed while hashing")?;
    ensure!(
        same_metadata(&opened, &pathname_after),
        "worktree file pathname changed while hashing"
    );
    let canonical_after =
        fs::canonicalize(&path).context("worktree file pathname could not be revalidated")?;
    ensure!(
        canonical_after == canonical,
        "worktree file pathname changed while hashing"
    );
    budget.file_bytes = budget
        .file_bytes
        .checked_add(before.len())
        .context("dirty snapshot byte count overflowed")?;
    ensure!(
        budget.file_bytes <= MAX_TOTAL_BYTES,
        "dirty snapshot total bytes exceed the supported limit"
    );
    hasher.field(b"worktree_kind", b"regular");
    hasher.field(b"worktree_mode", &(before.mode() & 0o7777).to_be_bytes());
    hasher.field(b"worktree_digest", &digest.finalize());
    Ok(())
}

fn verify_parent_within_root(root: &Path, path: &Path) -> Result<()> {
    let parent = path.parent().context("worktree path has no parent")?;
    let canonical =
        fs::canonicalize(parent).context("worktree path parent could not be verified")?;
    ensure!(
        canonical.starts_with(root),
        "worktree path escapes the checkout"
    );
    Ok(())
}

fn ensure_symlink_stays_within_root(root: &Path, parent: &Path, target: &Path) -> Result<()> {
    ensure!(
        !target.is_absolute(),
        "absolute symlink targets are unsupported"
    );
    let relative_parent = parent
        .strip_prefix(root)
        .context("symlink parent escapes the checkout")?;
    let mut depth = relative_parent.components().count();
    for component in target.components() {
        match component {
            Component::Normal(_) => depth = depth.saturating_add(1),
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir => bail!("symlink target escapes the checkout"),
            Component::RootDir | Component::Prefix(_) => {
                bail!("symlink target escapes the checkout")
            }
        }
    }
    Ok(())
}

fn same_metadata(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.mode() == right.mode()
        && left.nlink() == right.nlink()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

fn validate_challenge_identity(
    record: &ChallengeRecord,
    identity: &CheckoutIdentity,
    digest: &str,
) -> Result<()> {
    let valid = record.schema == CHALLENGE_SCHEMA
        && is_lower_hex(&record.token_digest, 64)
        && is_lower_hex(&record.session_key, 64)
        && is_lower_hex(&record.repository_key, 64)
        && is_lower_hex(&record.checkout_key, 64)
        && is_lower_hex(&record.checkout_instance, 32)
        && is_lower_hex(&record.snapshot_id, 64)
        && valid_head_identity(&record.head_oid)
        && is_lower_hex(&record.branch_ref_digest, 64)
        && is_lower_hex(&record.authorization_turn_digest, 64)
        && record.expires_at > record.issued_at
        && record.expires_at.saturating_sub(record.issued_at) <= 300;
    if !valid {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout challenge state is malformed",
        ));
    }
    if record.token_digest != digest {
        return Err(domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "dirty-checkout challenge token does not match",
        ));
    }
    if record.repository_key != identity.repository_key
        || record.checkout_key != identity.checkout_key
        || record.checkout_instance != identity.checkout_instance
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ChallengeDrift,
            "dirty-checkout challenge belongs to another checkout instance",
        ));
    }
    Ok(())
}

fn validate_challenge_at(record: &ChallengeRecord, now: u64) -> Result<()> {
    if record.issued_at > now || now >= record.expires_at {
        return Err(domain_error(
            DirtyCheckoutErrorKind::ChallengeExpired,
            "dirty-checkout challenge is expired or not yet valid",
        ));
    }
    Ok(())
}

fn validate_adoption_precommit_with<B, N>(
    challenge: &ChallengeRecord,
    identity: &CheckoutIdentity,
    before_validation: B,
    now: N,
) -> Result<()>
where
    B: FnOnce() -> Result<()>,
    N: FnOnce() -> Result<u64>,
{
    before_validation()?;
    validate_adoption_boundary(challenge, identity, now()?)
}

fn validate_adoption_boundary(
    challenge: &ChallengeRecord,
    identity: &CheckoutIdentity,
    now: u64,
) -> Result<()> {
    reject_active_git_operation(identity)?;
    validate_challenge_at(challenge, now)
}

fn valid_head_identity(value: &str) -> bool {
    if let Some(digest) = value.strip_prefix(UNBORN_HEAD_PREFIX) {
        return is_lower_hex(digest, 64);
    }
    (40..=64).contains(&value.len()) && is_lower_hex(value, value.len())
}

fn validate_receipt(
    record: &ReceiptRecord,
    identity: &CheckoutIdentity,
    receipt_id: &str,
) -> Result<()> {
    let valid = record.schema == RECEIPT_SCHEMA
        && is_lower_hex(&record.receipt_id, 64)
        && is_lower_hex(&record.session_key, 64)
        && is_lower_hex(&record.repository_key, 64)
        && is_lower_hex(&record.checkout_key, 64)
        && is_lower_hex(&record.checkout_instance, 32)
        && is_lower_hex(&record.snapshot_id, 64)
        && is_lower_hex(&record.authorization_turn_digest, 64)
        && is_lower_hex(&record.reason_digest, 64)
        && is_lower_hex(&record.challenge_digest, 64)
        && record.receipt_id == receipt_id
        && record.repository_key == identity.repository_key
        && record.checkout_key == identity.checkout_key
        && record.checkout_instance == identity.checkout_instance;
    if !valid {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout receipt state is malformed or belongs to another checkout",
        ));
    }
    Ok(())
}

fn load_lease(path: &Path) -> Result<Option<LeaseRecord>> {
    let bytes = match read_private_regular(path, MAX_STATE_FILE_BYTES, true) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|io| io.kind() == io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(command_error(
                error,
                DirtyCheckoutErrorKind::MalformedState,
                "checkout lease state is untrusted",
            ));
        }
    };
    Ok(Some(parse_lease(&bytes)?))
}

fn parse_lease(bytes: &[u8]) -> Result<LeaseRecord> {
    let wire: LeaseWire = serde_json::from_slice(bytes).map_err(|_| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "checkout lease state is malformed",
        )
    })?;
    match wire {
        LeaseWire::V1(wire) if wire.schema == LEASE_V1_SCHEMA => Ok(LeaseRecord::V1(wire)),
        LeaseWire::V2(wire) if wire.schema == LEASE_V2_SCHEMA => Ok(LeaseRecord::V2(wire)),
        LeaseWire::V1(_) | LeaseWire::V2(_) => Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "checkout lease schema is unsupported",
        )),
    }
}

fn validate_lease(lease: &LeaseRecord, identity: &CheckoutIdentity) -> Result<()> {
    if !is_lower_hex(lease.session_key(), 64)
        || !is_lower_hex(lease.checkout_instance(), 32)
        || lease.acquired_at() > lease.refreshed_at()
        || lease.refreshed_at() > lease.expires_at()
        || lease.checkout_instance() != identity.checkout_instance
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            "checkout lease identity or timestamps are malformed",
        ));
    }
    match lease {
        LeaseRecord::V1(lease) => {
            validate_lease_text_path(&lease.checkout_root, &identity.root, "checkout lease root")?;
            validate_lease_text_path(
                &lease.checkout_git_dir,
                &identity.git_dir,
                "checkout lease Git directory",
            )?;
            if lease.schema != LEASE_V1_SCHEMA {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "checkout lease schema is unsupported",
                ));
            }
        }
        LeaseRecord::V2(lease) => {
            if lease.schema != LEASE_V2_SCHEMA {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "checkout lease schema is unsupported",
                ));
            }
            let root_bytes = validate_lease_raw_path(
                &lease.checkout_root_bytes,
                &lease.checkout_root,
                &identity.root,
                "v2 checkout root identity",
            )?;
            let git_dir_bytes = validate_lease_raw_path(
                &lease.checkout_git_dir_bytes,
                &lease.checkout_git_dir,
                &identity.git_dir,
                "v2 checkout Git-dir identity",
            )?;
            ensure!(
                !root_bytes.is_empty() && !git_dir_bytes.is_empty(),
                "v2 checkout paths are malformed"
            );
            let adoption = &lease.adoption;
            let valid_adoption = adoption.schema == ADOPTION_SCHEMA
                && adoption.receipt_schema == RECEIPT_SCHEMA
                && is_lower_hex(&adoption.receipt_id, 64)
                && is_lower_hex(&adoption.snapshot_id, 64)
                && is_lower_hex(&adoption.authorization_turn_digest, 64)
                && is_lower_hex(&adoption.reason_digest, 64)
                && is_lower_hex(&adoption.challenge_digest, 64)
                && adoption.challenge_issued_at <= adoption.adopted_at
                && adoption.adopted_at <= lease.refreshed_at;
            if !valid_adoption {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::MalformedState,
                    "dirty-checkout adoption state is malformed",
                ));
            }
        }
    }
    Ok(())
}

fn validate_lease_text_path(value: &str, expected: &Path, label: &str) -> Result<()> {
    if value.is_empty()
        || value.as_bytes().contains(&0)
        || !Path::new(value).is_absolute()
        || value.as_bytes() != expected.as_os_str().as_bytes()
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            format!("{label} is malformed or does not match the current checkout"),
        ));
    }
    Ok(())
}

fn lease_path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn validate_lease_raw_path(
    value: &str,
    text: &str,
    expected: &Path,
    label: &str,
) -> Result<Vec<u8>> {
    let bytes = decode_lower_hex(value).ok_or_else(|| {
        domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            format!("{label} is malformed"),
        )
    })?;
    let path = Path::new(OsStr::from_bytes(&bytes));
    if bytes.is_empty()
        || bytes.contains(&0)
        || !path.is_absolute()
        || text != lease_path_text(expected)
        || bytes.as_slice() != expected.as_os_str().as_bytes()
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::MalformedState,
            format!("{label} does not match the compatible text or current checkout"),
        ));
    }
    Ok(bytes)
}

fn decode_lower_hex(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) || !is_lower_hex(value, value.len()) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let high = (pair[0] as char).to_digit(16)? as u8;
        let low = (pair[1] as char).to_digit(16)? as u8;
        bytes.push((high << 4) | low);
    }
    Some(bytes)
}

fn checkout_state_dir(state_root: &Path, identity: &CheckoutIdentity) -> Result<PathBuf> {
    validate_state_root(state_root, identity)?;
    let repository_dir = state_root.join(&identity.repository_key);
    let directory = repository_dir.join(&identity.checkout_key);
    verify_private_directory(&repository_dir).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout repository state directory is untrusted",
        )
    })?;
    verify_private_directory(&directory).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::MalformedState,
            "dirty-checkout checkout state directory is untrusted",
        )
    })?;
    verify_no_symlink_components(&directory)?;
    Ok(directory)
}

fn validate_state_root(state_root: &Path, identity: &CheckoutIdentity) -> Result<()> {
    if !state_root.is_absolute() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "dirty-checkout state root must be absolute",
        ));
    }
    verify_no_symlink_components(state_root)?;
    let canonical = fs::canonicalize(state_root).map_err(|error| {
        domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            format!("dirty-checkout state root is unavailable: {error}"),
        )
    })?;
    if canonical != state_root {
        return Err(domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "dirty-checkout state root must be canonical",
        ));
    }
    verify_private_directory(state_root).map_err(|error| {
        command_error(
            error,
            DirtyCheckoutErrorKind::InvalidInput,
            "dirty-checkout state root is not private",
        )
    })?;
    if canonical.starts_with(&identity.root)
        || canonical.starts_with(&identity.git_dir)
        || canonical.starts_with(&identity.common_dir)
    {
        return Err(domain_error(
            DirtyCheckoutErrorKind::InvalidInput,
            "dirty-checkout state root must be outside the checkout and Git directories",
        ));
    }
    Ok(())
}

fn verify_no_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                format!("state path component is unavailable: {error}"),
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(domain_error(
                DirtyCheckoutErrorKind::InvalidInput,
                "dirty-checkout state path contains a symlink component",
            ));
        }
    }
    Ok(())
}

fn private_directory(path: &Path) -> Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error).context("dirty-checkout state directory is unavailable");
        }
    }
    verify_private_directory(path)
}

fn verify_private_directory(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).context("dirty-checkout state directory is unavailable")?;
    ensure!(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        "dirty-checkout state path is not a trusted directory"
    );
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() },
        "dirty-checkout state directory has the wrong owner"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "dirty-checkout state directory is not private"
    );
    Ok(())
}

fn read_private_regular(path: &Path, max_bytes: usize, require_private: bool) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(anyhow::Error::new)?;
    let metadata = file
        .metadata()
        .context("state file metadata is unavailable")?;
    ensure!(
        metadata.file_type().is_file() && metadata.len() <= max_bytes as u64,
        "state file is not a bounded regular file"
    );
    if require_private {
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "state file has the wrong owner"
        );
        ensure!(metadata.mode() & 0o077 == 0, "state file is not private");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .context("state file read failed")?;
    ensure!(
        bytes.len() <= max_bytes,
        "state file exceeds the supported limit"
    );
    Ok(bytes)
}

fn read_regular_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .context("adoption reason file could not be opened safely")?;
    let metadata = file
        .metadata()
        .context("adoption reason metadata is unavailable")?;
    ensure!(
        metadata.file_type().is_file() && metadata.len() <= max_bytes as u64,
        "adoption reason is not a bounded regular file"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .context("adoption reason read failed")?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= max_bytes,
        "adoption reason file is empty or too large"
    );
    Ok(bytes)
}

fn write_json_atomic(path: &Path, value: &impl Serialize, replace: bool) -> Result<()> {
    write_json_atomic_with_limit(path, value, replace, MAX_STATE_FILE_BYTES)
}

fn write_json_atomic_with_limit(
    path: &Path,
    value: &impl Serialize,
    replace: bool,
    max_bytes: usize,
) -> Result<()> {
    let mut payload = serde_json::to_vec(value).context("state serialization failed")?;
    payload.push(b'\n');
    write_state_atomic_with_limit(path, &payload, replace, max_bytes)
}

fn write_state_atomic(path: &Path, payload: &[u8], replace: bool) -> Result<()> {
    write_state_atomic_with_limit(path, payload, replace, MAX_STATE_FILE_BYTES)
}

fn write_state_atomic_with_limit(
    path: &Path,
    payload: &[u8],
    replace: bool,
    max_bytes: usize,
) -> Result<()> {
    ensure!(
        !payload.is_empty() && payload.len() <= max_bytes,
        "serialized state exceeds the supported limit"
    );
    let parent = path
        .parent()
        .context("state file has no parent directory")?;
    let temporary = parent.join(format!(".state-{}", random_hex(16)?));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&temporary)
        .context("temporary state file could not be created")?;
    let result = (|| -> Result<()> {
        file.write_all(payload).context("state file write failed")?;
        file.sync_all().context("state file sync failed")?;
        drop(file);
        if replace {
            fs::rename(&temporary, path).context("state file replacement failed")?;
        } else {
            fs::hard_link(&temporary, path).context("state file creation failed")?;
            fs::remove_file(&temporary).context("temporary state cleanup failed")?;
        }
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn atomic_create_once(path: &Path, payload: &[u8]) -> Result<()> {
    let parent = path.parent().context("sentinel path has no parent")?;
    let temporary = parent.join(format!(".{INSTANCE_FILE}-{}", random_hex(8)?));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&temporary)
        .context("checkout instance temporary file could not be created")?;
    file.write_all(payload)
        .context("checkout instance write failed")?;
    file.sync_all().context("checkout instance sync failed")?;
    drop(file);
    match fs::hard_link(&temporary, path) {
        Ok(()) => sync_directory(parent)?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error).context("checkout instance sentinel could not be installed");
        }
    }
    fs::remove_file(&temporary).context("checkout instance temporary cleanup failed")?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .context("state directory sync failed")
}

struct LeaseLock(File);

mod removal_lease;
pub(super) use removal_lease::{
    RemovalLeaseGuard, fence_removal, removal_probe, removal_registry_lock,
};

impl LeaseLock {
    fn acquire_until(directory: &Path, transaction_deadline: Instant) -> Result<Self> {
        Self::acquire_until_with(directory, transaction_deadline, || {})
    }

    fn acquire_until_with<F>(
        directory: &Path,
        transaction_deadline: Instant,
        mut on_contention: F,
    ) -> Result<Self>
    where
        F: FnMut(),
    {
        ensure_deadline(transaction_deadline)?;
        let path = directory.join("lease.lock");
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| {
                domain_error(
                    DirtyCheckoutErrorKind::ResourceUnavailable,
                    format!("checkout lease lock is unavailable: {error}"),
                )
            })?;
        let metadata = file.metadata().map_err(|error| {
            domain_error(
                DirtyCheckoutErrorKind::ResourceUnavailable,
                format!("checkout lease lock metadata is unavailable: {error}"),
            )
        })?;
        if !metadata.file_type().is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(domain_error(
                DirtyCheckoutErrorKind::MalformedState,
                "checkout lease lock is not a private owner-controlled regular file",
            ));
        }
        loop {
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result == 0 {
                return Ok(Self(file));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::ResourceUnavailable,
                    format!("checkout lease lock failed: {error}"),
                ));
            }
            on_contention();
            if Instant::now() >= transaction_deadline {
                return Err(domain_error(
                    DirtyCheckoutErrorKind::Timeout,
                    "checkout lease lock timed out",
                ));
            }
            std::thread::sleep(LOCK_POLL);
        }
    }
}

impl Drop for LeaseLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd};

fn unix_time() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock precedes the Unix epoch")?
        .as_secs())
}

fn lease_ttl_seconds() -> u64 {
    env::var("AGENT_RUNTIME_CHECKOUT_LEASE_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(60, MAX_LEASE_TTL_SECONDS))
        .unwrap_or(DEFAULT_LEASE_TTL_SECONDS)
}

fn random_hex(byte_count: usize) -> Result<String> {
    let mut bytes = vec![0_u8; byte_count];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("secure random generation failed: {error}"))?;
    Ok(hex_bytes(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn strip_git_line(raw: &[u8]) -> Result<&[u8]> {
    ensure!(
        !raw.contains(&0),
        "Git output contains an unexpected NUL byte"
    );
    let without_lf = raw.strip_suffix(b"\n").unwrap_or(raw);
    Ok(without_lf.strip_suffix(b"\r").unwrap_or(without_lf))
}

fn deadline_after(timeout: Duration) -> Result<Instant> {
    if timeout.is_zero() {
        return Err(domain_error(
            DirtyCheckoutErrorKind::Timeout,
            "dirty snapshot exceeded the supported time limit",
        ));
    }
    Instant::now().checked_add(timeout).ok_or_else(|| {
        domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "dirty snapshot deadline could not be represented",
        )
    })
}

fn ensure_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(domain_error(
            DirtyCheckoutErrorKind::Timeout,
            "dirty snapshot exceeded the supported time limit",
        ));
    }
    Ok(())
}

fn git_output(
    cwd: &Path,
    args: &[&str],
    limit: usize,
    deadline: Instant,
) -> Result<std::process::Output> {
    let owned: Vec<OsString> = args.iter().map(OsString::from).collect();
    git_output_os(cwd, &owned, limit, deadline)
}

fn git_output_os(
    cwd: &Path,
    args: &[OsString],
    limit: usize,
    deadline: Instant,
) -> Result<std::process::Output> {
    ensure_deadline(deadline)?;
    let command_deadline = deadline.min(deadline_after(GIT_TIMEOUT)?);
    let executable = trusted_git_executable()?;
    ensure_deadline(deadline)?;
    let mut command = Command::new(executable);
    sanitize_git_environment(&mut command);
    command
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.trustctime=true")
        .arg("-c")
        .arg("core.checkStat=default")
        .arg("-c")
        .arg("core.fileMode=true")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if env::var_os(SNAPSHOT_WORKER_ENV).as_deref() != Some(OsStr::new("1")) {
        command.process_group(0);
    }
    output_with_aggregate_limit_until(
        &mut command,
        command_deadline,
        limit,
        MAX_GIT_STDERR_BYTES,
        limit,
    )
}

fn trusted_git_executable() -> Result<PathBuf> {
    for candidate in TRUSTED_GIT_PATHS {
        let path = Path::new(candidate);
        let Ok(canonical) = fs::canonicalize(path) else {
            continue;
        };
        let Ok(metadata) = fs::metadata(&canonical) else {
            continue;
        };
        if metadata.file_type().is_file()
            && trusted_system_git_owner(metadata.uid())
            && metadata.mode() & 0o022 == 0
            && metadata.mode() & 0o111 != 0
        {
            return Ok(canonical);
        }
    }
    Err(domain_error(
        DirtyCheckoutErrorKind::ResourceUnavailable,
        "a trusted system Git executable is unavailable",
    ))
}

fn trusted_system_git_owner(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }

    #[cfg(target_os = "linux")]
    {
        let Ok(overflow_uid) = read_linux_overflow_uid() else {
            return false;
        };
        let Ok(uid_map) =
            read_bounded_proc_text(Path::new(LINUX_UID_MAP_PATH), MAX_LINUX_UID_MAP_BYTES)
        else {
            return false;
        };
        linux_overflow_uid_proves_unmapped_host_root(uid, overflow_uid, &uid_map)
    }

    #[cfg(not(target_os = "linux"))]
    false
}

#[cfg(target_os = "linux")]
fn read_linux_overflow_uid() -> io::Result<u32> {
    let value = read_bounded_proc_text(Path::new(LINUX_OVERFLOW_UID_PATH), 32)?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    if value.is_empty() || value.bytes().any(|byte| !byte.is_ascii_digit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "kernel overflow UID is malformed",
        ));
    }
    value.parse::<u32>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "kernel overflow UID is out of range",
        )
    })
}

#[cfg(target_os = "linux")]
fn read_bounded_proc_text(path: &Path, limit: usize) -> io::Result<String> {
    let mut input = File::open(path)?.take(limit.saturating_add(1) as u64);
    let mut value = String::new();
    input.read_to_string(&mut value)?;
    if value.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "kernel identity metadata exceeds its supported size",
        ));
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
fn linux_overflow_uid_proves_unmapped_host_root(
    file_uid: u32,
    overflow_uid: u32,
    uid_map: &str,
) -> bool {
    if overflow_uid == 0 || file_uid != overflow_uid {
        return false;
    }

    let mut saw_mapping = false;
    for line in uid_map.lines() {
        if line.is_empty() {
            return false;
        }
        let mut fields = line.split_ascii_whitespace();
        let (Some(inside_start), Some(outside_start), Some(length)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return false;
        };
        if fields.next().is_some() {
            return false;
        }
        let (Some(inside_start), Some(outside_start), Some(length)) = (
            parse_linux_uid_map_field(inside_start),
            parse_linux_uid_map_field(outside_start),
            parse_linux_uid_map_field(length),
        ) else {
            return false;
        };
        let (Some(inside_end), Some(_outside_end)) = (
            inside_start.checked_add(length),
            outside_start.checked_add(length),
        ) else {
            return false;
        };
        if length == 0 {
            return false;
        }
        saw_mapping = true;
        let overflow_uid = u64::from(overflow_uid);
        if outside_start == 0 || (inside_start <= overflow_uid && overflow_uid < inside_end) {
            return false;
        }
    }

    saw_mapping
}

#[cfg(target_os = "linux")]
fn parse_linux_uid_map_field(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u64>().ok())
        .flatten()
}

pub(super) fn sanitize_git_environment(command: &mut Command) {
    const EXACT: &[&str] = &[
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_ASKPASS",
        "GIT_ATTR_NOSYSTEM",
        "GIT_CEILING_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_DIFF_OPTS",
        "GIT_DIR",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_EXEC_PATH",
        "GIT_EXTERNAL_DIFF",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_NAMESPACE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_QUARANTINE_PATH",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_WORK_TREE",
    ];
    for (key, _) in env::vars_os() {
        let name = key.to_string_lossy();
        if EXACT.contains(&name.as_ref())
            || name.starts_with("GIT_CONFIG")
            || name.starts_with("GIT_TRACE")
        {
            command.env_remove(key);
        }
    }
}

#[cfg(test)]
fn output_with_limits(
    command: &mut Command,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<std::process::Output> {
    output_with_aggregate_limit(
        command,
        timeout,
        stdout_limit,
        stderr_limit,
        stdout_limit.max(stderr_limit),
    )
}

#[cfg(test)]
fn output_with_aggregate_limit(
    command: &mut Command,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    aggregate_limit: usize,
) -> Result<std::process::Output> {
    output_with_aggregate_limit_until(
        command,
        deadline_after(timeout)?,
        stdout_limit,
        stderr_limit,
        aggregate_limit,
    )
}

struct SpawnedOutputChild {
    child: Child,
    supervised: bool,
    cleanup_proof: Option<std::os::unix::net::UnixStream>,
    fallback_owner: Option<ProcessOwner>,
}

fn spawn_output_child(command: &mut Command, deadline: Instant) -> Result<SpawnedOutputChild> {
    let is_snapshot_worker = command.get_envs().any(|(name, value)| {
        name == OsStr::new(SNAPSHOT_WORKER_ENV) && value == Some(OsStr::new("1"))
    });
    if is_snapshot_worker {
        return command
            .spawn()
            .map(|child| SpawnedOutputChild {
                child,
                supervised: false,
                cleanup_proof: None,
                fallback_owner: None,
            })
            .context("Git command could not be started");
    }
    let running_git_cli = env::var_os(SNAPSHOT_WORKER_ENV).as_deref() == Some(OsStr::new("1"))
        || env::current_exe().is_ok_and(|path| path.file_name() == Some(OsStr::new("git-cli")));
    if !running_git_cli {
        return command
            .spawn()
            .map(|child| SpawnedOutputChild {
                child,
                supervised: false,
                cleanup_proof: None,
                fallback_owner: None,
            })
            .context("Git command could not be started");
    }

    let executable = snapshot_worker_executable()?;
    spawn_supervisor_output_child(command, &executable, deadline)
}

fn spawn_supervisor_output_child(
    command: &mut Command,
    executable: &SnapshotWorkerExecutable,
    deadline: Instant,
) -> Result<SpawnedOutputChild> {
    let program = command.get_program().to_os_string();
    let arguments: Vec<OsString> = command.get_args().map(OsString::from).collect();
    let environment: Vec<(OsString, Option<OsString>)> = command
        .get_envs()
        .map(|(name, value)| (name.to_os_string(), value.map(OsString::from)))
        .collect();
    let current_dir = command.get_current_dir().map(PathBuf::from);
    executable.revalidate()?;
    let deadline_nanos = monotonic_deadline_nanos(deadline)?;
    let (mut capability_sender, capability_receiver) =
        std::os::unix::net::UnixStream::pair().context("process supervisor capability failed")?;
    let mut supervisor = Command::new(executable.command_path());
    supervisor
        .arg(program)
        .args(arguments)
        .stdin(Stdio::from(OwnedFd::from(capability_receiver)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(current_dir) = current_dir {
        supervisor.current_dir(current_dir);
    }
    for (name, value) in environment {
        match value {
            Some(value) => {
                supervisor.env(name, value);
            }
            None => {
                supervisor.env_remove(name);
            }
        }
    }
    supervisor.env(PROCESS_SUPERVISOR_ENV, "1").env(
        PROCESS_SUPERVISOR_CAPABILITY_FD_ENV,
        libc::STDIN_FILENO.to_string(),
    );
    let mut child = supervisor
        .spawn()
        .context("Git command could not be started")?;
    let fallback_owner = match ProcessOwner::for_root(child.id() as libc::pid_t) {
        Ok(owner) => owner,
        Err(error) => {
            unsafe {
                let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if let Err(error) = capability_sender.write_all(PROCESS_SUPERVISOR_CAPABILITY) {
        unsafe {
            let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("process supervisor capability could not be delivered");
    }
    if let Err(error) = capability_sender.write_all(&deadline_nanos.to_be_bytes()) {
        unsafe {
            let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("process supervisor deadline could not be delivered");
    }
    Ok(SpawnedOutputChild {
        child,
        supervised: true,
        cleanup_proof: Some(capability_sender),
        fallback_owner: Some(fallback_owner),
    })
}

fn output_with_aggregate_limit_until(
    command: &mut Command,
    deadline: Instant,
    stdout_limit: usize,
    stderr_limit: usize,
    aggregate_limit: usize,
) -> Result<std::process::Output> {
    ensure_deadline(deadline)?;
    let child = spawn_output_child(command, deadline)?;
    collect_output_child_until(child, deadline, stdout_limit, stderr_limit, aggregate_limit)
}

fn collect_output_child_until(
    spawned: SpawnedOutputChild,
    deadline: Instant,
    stdout_limit: usize,
    stderr_limit: usize,
    aggregate_limit: usize,
) -> Result<std::process::Output> {
    let SpawnedOutputChild {
        mut child,
        supervised,
        mut cleanup_proof,
        mut fallback_owner,
    } = spawned;
    let process_group = child.id() as libc::pid_t;
    if Instant::now() >= deadline {
        terminate_child(
            &mut child,
            process_group,
            supervised,
            &mut cleanup_proof,
            &mut fallback_owner,
        );
        return Err(domain_error(
            DirtyCheckoutErrorKind::Timeout,
            "Git command exceeded the supported time limit",
        ));
    }
    let mut stdout = child.stdout.take().context("Git stdout was not captured")?;
    let mut stderr = child.stderr.take().context("Git stderr was not captured")?;
    if let Err(error) =
        set_nonblocking(stdout.as_raw_fd()).and_then(|()| set_nonblocking(stderr.as_raw_fd()))
    {
        terminate_child(
            &mut child,
            process_group,
            supervised,
            &mut cleanup_proof,
            &mut fallback_owner,
        );
        return Err(domain_error(
            DirtyCheckoutErrorKind::ResourceUnavailable,
            format!("Git output pipes could not be made nonblocking: {error}"),
        ));
    }

    let mut stdout_bytes = Vec::with_capacity(stdout_limit.min(64 * 1024));
    let mut stderr_bytes = Vec::with_capacity(stderr_limit.min(64 * 1024));
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    loop {
        if let Some(owner) = &mut fallback_owner
            && let Err(error) = owner.identities(deadline)
        {
            terminate_child(
                &mut child,
                process_group,
                supervised,
                &mut cleanup_proof,
                &mut fallback_owner,
            );
            return Err(error);
        }
        if stdout_eof && stderr_eof {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if supervised {
                        let completion = cleanup_proof
                            .as_mut()
                            .ok_or_else(process_scan_resource_error)
                            .and_then(read_process_supervisor_completion);
                        let valid_target = completion.is_ok_and(|completion| {
                            completion.kind == ProcessSupervisorCompletionKind::Target
                                && completion.cleanup_complete
                                && status.code() == Some(completion.exit_code)
                        });
                        if !valid_target {
                            terminate_child(
                                &mut child,
                                process_group,
                                supervised,
                                &mut cleanup_proof,
                                &mut fallback_owner,
                            );
                            return Err(domain_error(
                                DirtyCheckoutErrorKind::ResourceUnavailable,
                                "process supervisor completion could not be authenticated",
                            ));
                        }
                    }
                    return Ok(std::process::Output {
                        status,
                        stdout: stdout_bytes,
                        stderr: stderr_bytes,
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    terminate_child(
                        &mut child,
                        process_group,
                        supervised,
                        &mut cleanup_proof,
                        &mut fallback_owner,
                    );
                    return Err(error).context("Git command status failed");
                }
            }
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate_child(
                &mut child,
                process_group,
                supervised,
                &mut cleanup_proof,
                &mut fallback_owner,
            );
            return Err(domain_error(
                DirtyCheckoutErrorKind::Timeout,
                "Git command exceeded the supported time limit",
            ));
        }
        if stdout_eof && stderr_eof {
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
            continue;
        }

        let mut poll_fds = [
            libc::pollfd {
                fd: if stdout_eof { -1 } else { stdout.as_raw_fd() },
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            },
            libc::pollfd {
                fd: if stderr_eof { -1 } else { stderr.as_raw_fd() },
                events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                revents: 0,
            },
        ];
        let poll_timeout = duration_to_poll_timeout(remaining.min(Duration::from_millis(10)));
        let polled = unsafe {
            libc::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as libc::nfds_t,
                poll_timeout,
            )
        };
        if polled < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            terminate_child(
                &mut child,
                process_group,
                supervised,
                &mut cleanup_proof,
                &mut fallback_owner,
            );
            return Err(error).context("Git output polling failed");
        }

        let aggregate_before = stdout_bytes.len().saturating_add(stderr_bytes.len());
        if !stdout_eof && poll_fds[0].revents != 0 {
            match drain_nonblocking(
                &mut stdout,
                &mut stdout_bytes,
                stdout_limit,
                aggregate_limit.saturating_sub(aggregate_before),
            ) {
                Ok(eof) => stdout_eof = eof,
                Err(error) => {
                    terminate_child(
                        &mut child,
                        process_group,
                        supervised,
                        &mut cleanup_proof,
                        &mut fallback_owner,
                    );
                    return Err(error);
                }
            }
        }
        let aggregate_before = stdout_bytes.len().saturating_add(stderr_bytes.len());
        if !stderr_eof && poll_fds[1].revents != 0 {
            match drain_nonblocking(
                &mut stderr,
                &mut stderr_bytes,
                stderr_limit,
                aggregate_limit.saturating_sub(aggregate_before),
            ) {
                Ok(eof) => stderr_eof = eof,
                Err(error) => {
                    terminate_child(
                        &mut child,
                        process_group,
                        supervised,
                        &mut cleanup_proof,
                        &mut fallback_owner,
                    );
                    return Err(error);
                }
            }
        }
    }
}

fn set_nonblocking(fd: libc::c_int) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn duration_to_poll_timeout(duration: Duration) -> libc::c_int {
    let millis = duration.as_millis().max(1);
    libc::c_int::try_from(millis).unwrap_or(libc::c_int::MAX)
}

fn drain_nonblocking<R: Read>(
    reader: &mut R,
    bytes: &mut Vec<u8>,
    stream_limit: usize,
    aggregate_remaining: usize,
) -> Result<bool> {
    let mut aggregate_remaining = aggregate_remaining;
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(read) => {
                let stream_remaining = stream_limit.saturating_sub(bytes.len());
                if read > stream_remaining || read > aggregate_remaining {
                    return Err(domain_error(
                        DirtyCheckoutErrorKind::ResourceUnavailable,
                        "Git output exceeds the supported size limit",
                    ));
                }
                bytes.extend_from_slice(&buffer[..read]);
                aggregate_remaining -= read;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error).context("Git output read failed"),
        }
    }
}

// One cleanup phase may include macOS scheduler-delayed kqueue draining,
// descendant discovery, identity-pinned termination, and reaping. Two seconds
// gives those bounded phases practical headroom while keeping the supervised
// fallback below the dirty-snapshot wrapper budget: 30s snapshot +
// (2s + 25ms supervisor grace) + 2s fallback < 35s.
const PROCESS_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const PROCESS_SCAN_LIMITS: ProcessScanLimits = ProcessScanLimits {
    #[cfg(target_os = "linux")]
    process_entries: 32_768,
    #[cfg(target_os = "linux")]
    metadata_bytes: 32 * 1024 * 1024,
    descendants: 4_096,
};
const MAX_PROCESS_CLEANUP_GENERATIONS: usize = PROCESS_SCAN_LIMITS.descendants;
#[cfg(target_os = "linux")]
const MAX_PROC_STAT_BYTES: usize = 4 * 1024;
#[cfg(target_os = "linux")]
const MAX_PROC_CHILDREN_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ProcessIdentity {
    pid: libc::pid_t,
    generation: u128,
}

#[derive(Clone, Copy, Debug)]
struct ProcessRecord {
    identity: ProcessIdentity,
    parent: libc::pid_t,
    #[cfg(all(test, target_os = "linux"))]
    metadata_bytes: usize,
}

impl ProcessRecord {
    fn new(pid: libc::pid_t, parent: libc::pid_t, generation: u128, metadata_bytes: usize) -> Self {
        #[cfg(not(all(test, target_os = "linux")))]
        let _ = metadata_bytes;
        Self {
            identity: ProcessIdentity { pid, generation },
            parent,
            #[cfg(all(test, target_os = "linux"))]
            metadata_bytes,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ProcessScanLimits {
    #[cfg(target_os = "linux")]
    process_entries: usize,
    #[cfg(target_os = "linux")]
    metadata_bytes: usize,
    descendants: usize,
}

#[derive(Debug)]
struct ProcessScan {
    identities: Vec<ProcessIdentity>,
    #[cfg(all(test, target_os = "linux"))]
    edge_visits: usize,
}

fn process_scan_resource_error() -> anyhow::Error {
    domain_error(
        DirtyCheckoutErrorKind::ResourceUnavailable,
        "process cleanup exceeds the supported resource limit",
    )
}

#[cfg(all(test, target_os = "linux"))]
fn collect_descendant_processes_with<F>(
    root: libc::pid_t,
    deadline: Instant,
    limits: ProcessScanLimits,
    read_records: F,
) -> Result<ProcessScan>
where
    F: FnOnce(Instant) -> Result<Vec<ProcessRecord>>,
{
    ensure_deadline(deadline)?;
    let records = read_records(deadline)?;
    ensure_deadline(deadline)?;
    if records.len() > limits.process_entries {
        return Err(process_scan_resource_error());
    }

    let mut metadata_bytes = 0_usize;
    let mut children: std::collections::HashMap<libc::pid_t, Vec<ProcessIdentity>> =
        std::collections::HashMap::new();
    for record in records {
        ensure_deadline(deadline)?;
        metadata_bytes = metadata_bytes
            .checked_add(record.metadata_bytes)
            .ok_or_else(process_scan_resource_error)?;
        if metadata_bytes > limits.metadata_bytes {
            return Err(process_scan_resource_error());
        }
        children
            .entry(record.parent)
            .or_default()
            .push(record.identity);
    }

    let mut identities = Vec::new();
    let mut seen = HashSet::new();
    let mut frontier = vec![root];
    let mut edge_visits = 0_usize;
    while let Some(parent) = frontier.pop() {
        ensure_deadline(deadline)?;
        let Some(direct) = children.get(&parent) else {
            continue;
        };
        for identity in direct {
            edge_visits = edge_visits
                .checked_add(1)
                .ok_or_else(process_scan_resource_error)?;
            if seen.insert(*identity) {
                if identities.len() >= limits.descendants {
                    return Err(process_scan_resource_error());
                }
                identities.push(*identity);
                frontier.push(identity.pid);
            }
        }
    }
    Ok(ProcessScan {
        identities,
        #[cfg(all(test, target_os = "linux"))]
        edge_visits,
    })
}

#[cfg(target_os = "linux")]
fn pin_process_identity_result_with<H, O, V>(
    identity: ProcessIdentity,
    open: O,
    current_identity: V,
) -> Result<Option<H>>
where
    O: FnOnce(libc::pid_t) -> io::Result<H>,
    V: FnOnce(libc::pid_t) -> Result<Option<ProcessIdentity>>,
{
    let handle = match open(identity.pid) {
        Ok(handle) => handle,
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(None),
        Err(_) => return Err(process_scan_resource_error()),
    };
    Ok((current_identity(identity.pid)? == Some(identity)).then_some(handle))
}

#[cfg(target_os = "linux")]
fn parse_linux_process_record(bytes: &[u8]) -> Option<ProcessRecord> {
    let open = bytes.iter().position(|byte| *byte == b' ')?;
    let close = bytes.iter().rposition(|byte| *byte == b')')?;
    let pid = std::str::from_utf8(bytes.get(..open)?)
        .ok()?
        .parse::<libc::pid_t>()
        .ok()?;
    let mut fields = bytes
        .get(close + 1..)?
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    let parent = std::str::from_utf8(fields.nth(1)?)
        .ok()?
        .parse::<libc::pid_t>()
        .ok()?;
    let generation = std::str::from_utf8(fields.nth(17)?)
        .ok()?
        .parse::<u128>()
        .ok()?;
    Some(ProcessRecord::new(pid, parent, generation, bytes.len()))
}

/// Whether a `/proc` failure means the process simply went away.
///
/// The scan walks short-lived descendants, so a process departing mid-walk is
/// expected rather than exceptional. `openat` reports a departed process as
/// `ENOENT`, which Rust maps to `NotFound`. A process that exits between the
/// open and the read instead reports `ESRCH` on the read, and Rust maps that to
/// `Uncategorized`, so it cannot be recognized by `ErrorKind`. Both mean the
/// same thing: there is nothing left to scan, and neither is a resource limit.
#[cfg(target_os = "linux")]
fn is_departed_process_error(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ESRCH) | Some(libc::ENOENT))
}

#[cfg(target_os = "linux")]
fn read_linux_metadata(path: &Path, byte_limit: usize) -> Result<Option<Vec<u8>>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if is_departed_process_error(&error) => return Ok(None),
        Err(_) => return Err(process_scan_resource_error()),
    };
    read_open_linux_metadata(file, byte_limit)
}

/// Read one already-opened `/proc` entry, treating a process that departed
/// after the open as absent rather than as a resource failure.
#[cfg(target_os = "linux")]
fn read_open_linux_metadata(file: File, byte_limit: usize) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::with_capacity(256.min(byte_limit));
    match file.take((byte_limit + 1) as u64).read_to_end(&mut bytes) {
        Ok(_) => {}
        Err(error) if is_departed_process_error(&error) => return Ok(None),
        Err(_) => return Err(process_scan_resource_error()),
    }
    if bytes.len() > byte_limit {
        return Err(process_scan_resource_error());
    }
    Ok(Some(bytes))
}

#[cfg(target_os = "linux")]
fn charge_process_scan(counter: &mut usize, amount: usize, limit: usize) -> Result<()> {
    *counter = counter
        .checked_add(amount)
        .ok_or_else(process_scan_resource_error)?;
    if *counter > limit {
        return Err(process_scan_resource_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn read_linux_process_record_at(
    proc_root: &Path,
    pid: libc::pid_t,
    deadline: Instant,
    metadata_bytes: &mut usize,
    limits: ProcessScanLimits,
) -> Result<Option<ProcessRecord>> {
    ensure_deadline(deadline)?;
    let Some(bytes) = read_linux_metadata(
        &proc_root.join(pid.to_string()).join("stat"),
        MAX_PROC_STAT_BYTES,
    )?
    else {
        return Ok(None);
    };
    charge_process_scan(metadata_bytes, bytes.len(), limits.metadata_bytes)?;
    Ok(parse_linux_process_record(&bytes).filter(|record| record.identity.pid == pid))
}

#[cfg(target_os = "linux")]
fn linux_process_identity_matches_at(
    proc_root: &Path,
    expected: ProcessIdentity,
    deadline: Instant,
    metadata_bytes: &mut usize,
    limits: ProcessScanLimits,
) -> Result<bool> {
    Ok(
        read_linux_process_record_at(proc_root, expected.pid, deadline, metadata_bytes, limits)?
            .is_some_and(|record| record.identity == expected),
    )
}

#[cfg(target_os = "linux")]
fn visit_linux_child_identities_at<F>(
    proc_root: &Path,
    parent: ProcessIdentity,
    deadline: Instant,
    process_entries: &mut usize,
    metadata_bytes: &mut usize,
    limits: ProcessScanLimits,
    mut visit: F,
) -> Result<()>
where
    F: FnMut(ProcessIdentity) -> Result<()>,
{
    ensure_deadline(deadline)?;
    if !linux_process_identity_matches_at(proc_root, parent, deadline, metadata_bytes, limits)? {
        return Ok(());
    }
    let task_path = proc_root.join(parent.pid.to_string()).join("task");
    let tasks = match fs::read_dir(task_path) {
        Ok(tasks) => tasks,
        Err(error) if is_departed_process_error(&error) => return Ok(()),
        Err(_) => return Err(process_scan_resource_error()),
    };
    for task in tasks {
        ensure_deadline(deadline)?;
        let Ok(task) = task else {
            continue;
        };
        if task
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
            .is_none()
        {
            continue;
        }
        if !linux_process_identity_matches_at(proc_root, parent, deadline, metadata_bytes, limits)?
        {
            return Ok(());
        }
        charge_process_scan(process_entries, 1, limits.process_entries)?;
        let Some(bytes) =
            read_linux_metadata(&task.path().join("children"), MAX_PROC_CHILDREN_BYTES)?
        else {
            continue;
        };
        charge_process_scan(metadata_bytes, bytes.len(), limits.metadata_bytes)?;
        if !linux_process_identity_matches_at(proc_root, parent, deadline, metadata_bytes, limits)?
        {
            return Ok(());
        }
        let child_pids = bytes
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .map(|field| {
                ensure_deadline(deadline)?;
                let pid = std::str::from_utf8(field)
                    .ok()
                    .and_then(|field| field.parse::<libc::pid_t>().ok())
                    .ok_or_else(process_scan_resource_error)?;
                charge_process_scan(process_entries, 1, limits.process_entries)?;
                Ok(pid)
            });
        visit_identity_bound_children_with(
            parent,
            child_pids,
            |pid| read_linux_process_record_at(proc_root, pid, deadline, metadata_bytes, limits),
            &mut visit,
        )?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn descendant_processes_at<F>(
    proc_root: &Path,
    root: libc::pid_t,
    deadline: Instant,
    limits: ProcessScanLimits,
    mut observe: F,
) -> Result<ProcessScan>
where
    F: FnMut(ProcessIdentity),
{
    ensure_deadline(deadline)?;
    let mut identities = Vec::new();
    let mut seen = HashSet::new();
    let mut process_entries = 0_usize;
    let mut metadata_bytes = 0_usize;
    let mut edge_visits = 0_usize;
    let Some(root_identity) =
        read_linux_process_record_at(proc_root, root, deadline, &mut metadata_bytes, limits)?
            .map(|record| record.identity)
    else {
        return Ok(ProcessScan {
            identities,
            #[cfg(all(test, target_os = "linux"))]
            edge_visits,
        });
    };
    let mut frontier = vec![root_identity];
    while let Some(parent) = frontier.pop() {
        ensure_deadline(deadline)?;
        visit_linux_child_identities_at(
            proc_root,
            parent,
            deadline,
            &mut process_entries,
            &mut metadata_bytes,
            limits,
            |identity| {
                ensure_deadline(deadline)?;
                edge_visits = edge_visits
                    .checked_add(1)
                    .ok_or_else(process_scan_resource_error)?;
                if !seen.insert(identity) {
                    return Ok(());
                }
                if identities.len() >= limits.descendants {
                    return Err(process_scan_resource_error());
                }
                observe(identity);
                identities.push(identity);
                frontier.push(identity);
                Ok(())
            },
        )?;
    }
    Ok(ProcessScan {
        identities,
        #[cfg(all(test, target_os = "linux"))]
        edge_visits,
    })
}

#[cfg(target_os = "linux")]
fn descendant_processes_with<F>(
    root: libc::pid_t,
    deadline: Instant,
    observe: F,
) -> Result<ProcessScan>
where
    F: FnMut(ProcessIdentity),
{
    descendant_processes_at(
        Path::new("/proc"),
        root,
        deadline,
        PROCESS_SCAN_LIMITS,
        observe,
    )
}

#[cfg(target_os = "linux")]
fn process_identity(pid: libc::pid_t) -> Result<Option<ProcessIdentity>> {
    let Some(bytes) = read_linux_metadata(
        &Path::new("/proc").join(pid.to_string()).join("stat"),
        MAX_PROC_STAT_BYTES,
    )?
    else {
        return Ok(None);
    };
    let record = parse_linux_process_record(&bytes).ok_or_else(process_scan_resource_error)?;
    if record.identity.pid != pid {
        return Err(process_scan_resource_error());
    }
    Ok(Some(record.identity))
}

#[cfg(target_os = "linux")]
struct PinnedProcess {
    pidfd: File,
}

#[cfg(target_os = "linux")]
fn pin_process(identity: ProcessIdentity) -> Result<Option<PinnedProcess>> {
    use std::os::fd::FromRawFd;

    pin_process_identity_result_with(
        identity,
        |pid| {
            let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
            if pidfd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(unsafe { File::from_raw_fd(pidfd as libc::c_int) })
        },
        process_identity,
    )
    .map(|pinned| pinned.map(|pidfd| PinnedProcess { pidfd }))
}

#[cfg(target_os = "linux")]
fn signal_pinned_process_result_with<F>(send: F) -> Result<bool>
where
    F: FnOnce() -> io::Result<()>,
{
    match send() {
        Ok(()) => Ok(true),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(false),
        Err(_) => Err(process_scan_resource_error()),
    }
}

#[cfg(target_os = "linux")]
fn signal_pinned_process(process: &PinnedProcess, signal: libc::c_int) -> Result<bool> {
    signal_pinned_process_result_with(|| {
        let status = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                process.pidfd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0_u32,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    })
}

#[cfg(target_os = "linux")]
fn wait_pinned_process_until(process: &PinnedProcess, deadline: Instant) -> Result<()> {
    loop {
        ensure_deadline(deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout_millis = remaining.as_millis().min(libc::c_int::MAX as u128) as libc::c_int;
        let mut descriptor = libc::pollfd {
            fd: process.pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let status = unsafe { libc::poll(&mut descriptor, 1, timeout_millis) };
        if status > 0 {
            if descriptor.revents & libc::POLLIN != 0 {
                return Ok(());
            }
            return Err(process_scan_resource_error());
        }
        if status == 0 {
            continue;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(process_scan_resource_error());
        }
    }
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
fn visit_identity_bound_children_with<I, R, F>(
    parent: ProcessIdentity,
    children: I,
    mut read_record: R,
    mut visit: F,
) -> Result<()>
where
    I: IntoIterator<Item = Result<libc::pid_t>>,
    R: FnMut(libc::pid_t) -> Result<Option<ProcessRecord>>,
    F: FnMut(ProcessIdentity) -> Result<()>,
{
    for child in children {
        if read_record(parent.pid)?.map(|record| record.identity) != Some(parent) {
            return Ok(());
        }
        let pid = child?;
        let Some(record) = read_record(pid)? else {
            continue;
        };
        if record.parent != parent.pid {
            continue;
        }
        if read_record(parent.pid)?.map(|record| record.identity) != Some(parent) {
            return Ok(());
        }
        visit(record.identity)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_process_record(pid: libc::pid_t) -> Result<Option<ProcessRecord>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    unsafe {
        *libc::__error() = 0;
    }
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };
    if read == size as libc::c_int {
        let info = unsafe { info.assume_init() };
        return Ok((info.pbi_pid == pid as u32).then(|| {
            ProcessRecord::new(
                pid,
                info.pbi_ppid as libc::pid_t,
                ((info.pbi_start_tvsec as u128) << 64) | info.pbi_start_tvusec as u128,
                size,
            )
        }));
    }
    let error = io::Error::last_os_error();
    if read == 0 && error.raw_os_error() == Some(libc::ESRCH) {
        Ok(None)
    } else {
        Err(process_scan_resource_error())
    }
}

#[cfg(target_os = "macos")]
fn process_identity(pid: libc::pid_t) -> Result<Option<ProcessIdentity>> {
    Ok(macos_process_record(pid)?.map(|record| record.identity))
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn task_suspend(target_task: libc::mach_port_t) -> libc::kern_return_t;
    fn task_resume(target_task: libc::mach_port_t) -> libc::kern_return_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
    #[link_name = "mach_task_self_"]
    static CURRENT_MACH_TASK: libc::mach_port_t;
}

#[cfg(target_os = "macos")]
fn current_mach_task() -> libc::mach_port_t {
    unsafe { CURRENT_MACH_TASK }
}

#[cfg(target_os = "macos")]
struct MachTaskPort(libc::mach_port_t);

#[cfg(target_os = "macos")]
impl Drop for MachTaskPort {
    fn drop(&mut self) {
        unsafe {
            let _ = mach_port_deallocate(current_mach_task(), self.0);
        }
    }
}

#[cfg(target_os = "macos")]
struct PinnedProcess {
    identity: ProcessIdentity,
    task: MachTaskPort,
}

#[cfg(target_os = "macos")]
fn pin_process(identity: ProcessIdentity) -> Result<Option<PinnedProcess>> {
    let mut task = 0;
    let status = unsafe { libc::task_for_pid(current_mach_task(), identity.pid, &mut task) };
    if status != libc::KERN_SUCCESS {
        return match process_identity(identity.pid)? {
            None => Ok(None),
            Some(_) => Err(process_scan_resource_error()),
        };
    }
    let task = MachTaskPort(task);
    if process_identity(identity.pid)? != Some(identity) {
        return Ok(None);
    }
    Ok(Some(PinnedProcess { identity, task }))
}

#[cfg(target_os = "macos")]
fn signal_pinned_process(process: &PinnedProcess, signal: libc::c_int) -> Result<bool> {
    let status = match signal {
        libc::SIGSTOP => unsafe { task_suspend(process.task.0) },
        libc::SIGCONT => unsafe { task_resume(process.task.0) },
        libc::SIGKILL => unsafe { libc::task_terminate(process.task.0) },
        _ => return Err(process_scan_resource_error()),
    };
    if status == libc::KERN_SUCCESS {
        return Ok(true);
    }
    if process_identity(process.identity.pid)? != Some(process.identity) {
        Ok(false)
    } else {
        Err(process_scan_resource_error())
    }
}

#[cfg(target_os = "macos")]
fn wait_pinned_process_until(process: &PinnedProcess, deadline: Instant) -> Result<()> {
    loop {
        if process_identity(process.identity.pid)? != Some(process.identity) {
            return Ok(());
        }
        ensure_deadline(deadline)?;
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(2)),
        );
    }
}

#[cfg(any(test, target_os = "macos"))]
fn macos_descendant_processes_from_identity_with<R, L, F>(
    root: ProcessIdentity,
    deadline: Instant,
    mut read_record: R,
    mut list_children: L,
    mut observe: F,
) -> Result<ProcessScan>
where
    R: FnMut(libc::pid_t) -> Result<Option<ProcessRecord>>,
    L: FnMut(libc::pid_t, &mut [libc::pid_t]) -> Result<usize>,
    F: FnMut(ProcessIdentity),
{
    let mut identities = Vec::new();
    let mut seen = HashSet::new();
    let mut edge_visits = 0_usize;
    let mut frontier = vec![root];
    let mut children = vec![0 as libc::pid_t; PROCESS_SCAN_LIMITS.descendants + 1];
    while let Some(parent) = frontier.pop() {
        ensure_deadline(deadline)?;
        if read_record(parent.pid)?.map(|record| record.identity) != Some(parent) {
            continue;
        }
        let count = list_children(parent.pid, &mut children)?;
        if read_record(parent.pid)?.map(|record| record.identity) != Some(parent) {
            continue;
        }
        if count == 0 {
            continue;
        }
        if count > PROCESS_SCAN_LIMITS.descendants {
            return Err(process_scan_resource_error());
        }
        visit_identity_bound_children_with(
            parent,
            children.iter().take(count).copied().map(Ok),
            |pid| {
                ensure_deadline(deadline)?;
                if pid != parent.pid {
                    edge_visits = edge_visits
                        .checked_add(1)
                        .ok_or_else(process_scan_resource_error)?;
                }
                read_record(pid)
            },
            |identity| {
                if seen.insert(identity) {
                    if identities.len() >= PROCESS_SCAN_LIMITS.descendants {
                        return Err(process_scan_resource_error());
                    }
                    observe(identity);
                    identities.push(identity);
                    frontier.push(identity);
                }
                Ok(())
            },
        )?;
    }
    Ok(ProcessScan {
        identities,
        #[cfg(all(test, target_os = "linux"))]
        edge_visits,
    })
}

#[cfg(target_os = "macos")]
fn descendant_processes_from_identity_with<F>(
    root: ProcessIdentity,
    deadline: Instant,
    observe: F,
) -> Result<ProcessScan>
where
    F: FnMut(ProcessIdentity),
{
    macos_descendant_processes_from_identity_with(
        root,
        deadline,
        macos_process_record,
        |parent, children| {
            unsafe {
                *libc::__error() = 0;
            }
            let count = unsafe {
                libc::proc_listchildpids(
                    parent,
                    children.as_mut_ptr().cast(),
                    std::mem::size_of_val(children) as libc::c_int,
                )
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    return Ok(0);
                }
                return Err(process_scan_resource_error());
            }
            Ok(count as usize)
        },
        observe,
    )
}

#[cfg(target_os = "macos")]
fn descendant_processes_from_identity(
    root: ProcessIdentity,
    deadline: Instant,
) -> Result<ProcessScan> {
    descendant_processes_from_identity_with(root, deadline, |_| {})
}

#[cfg(target_os = "macos")]
fn descendant_processes_with<F>(
    root: libc::pid_t,
    deadline: Instant,
    observe: F,
) -> Result<ProcessScan>
where
    F: FnMut(ProcessIdentity),
{
    let Some(root_identity) = process_identity(root)? else {
        return Ok(ProcessScan {
            identities: Vec::new(),
            #[cfg(all(test, target_os = "linux"))]
            edge_visits: 0,
        });
    };
    descendant_processes_from_identity_with(root_identity, deadline, observe)
}

#[cfg(any(test, target_os = "macos"))]
fn macos_fallback_descendants_with<R, S>(
    root: ProcessIdentity,
    deadline: Instant,
    mut current_identity: R,
    mut scan_descendants: S,
) -> Result<ProcessScan>
where
    R: FnMut(libc::pid_t) -> Result<Option<ProcessIdentity>>,
    S: FnMut(ProcessIdentity, Instant) -> Result<ProcessScan>,
{
    ensure_deadline(deadline)?;
    if current_identity(root.pid)? != Some(root) {
        return Ok(ProcessScan {
            identities: Vec::new(),
            #[cfg(all(test, target_os = "linux"))]
            edge_visits: 0,
        });
    }
    scan_descendants(root, deadline)
}

#[cfg(target_os = "linux")]
fn probe_pidfd_support() -> Result<()> {
    use std::os::fd::FromRawFd;

    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::getpid(), 0_u32) };
    if descriptor < 0 {
        return Err(process_scan_resource_error());
    }
    let pidfd = unsafe { File::from_raw_fd(descriptor as libc::c_int) };
    let status = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            0,
            std::ptr::null::<libc::siginfo_t>(),
            0_u32,
        )
    };
    if status != 0 {
        return Err(process_scan_resource_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct ProcessOwner {
    root: ProcessIdentity,
    reap_adopted: bool,
    known: HashSet<ProcessIdentity>,
}

#[cfg(target_os = "linux")]
impl ProcessOwner {
    fn new() -> Result<Self> {
        let root = unsafe { libc::getpid() };
        Self::with_root(root, true)
    }

    fn for_root(root: libc::pid_t) -> Result<Self> {
        Self::with_root(root, false)
    }

    fn with_root(root: libc::pid_t, reap_adopted: bool) -> Result<Self> {
        probe_pidfd_support()?;
        if reap_adopted && unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
            return Err(process_scan_resource_error());
        }
        let root = process_identity(root)?.ok_or_else(process_scan_resource_error)?;
        Ok(Self {
            root,
            reap_adopted,
            known: HashSet::new(),
        })
    }

    fn refresh(&mut self, deadline: Instant) -> Result<()> {
        ensure_deadline(deadline)
    }

    fn identities(&mut self, deadline: Instant) -> Result<Vec<ProcessIdentity>> {
        if process_identity(self.root.pid)? != Some(self.root) {
            return Ok(Vec::new());
        }
        let known = &mut self.known;
        let mut generation_limit_exceeded = false;
        let scan = descendant_processes_at(
            Path::new("/proc"),
            self.root.pid,
            deadline,
            PROCESS_SCAN_LIMITS,
            |identity| {
                if known.contains(&identity) {
                    return;
                }
                if known.len() >= MAX_PROCESS_CLEANUP_GENERATIONS {
                    generation_limit_exceeded = true;
                    return;
                }
                known.insert(identity);
            },
        )?;
        if generation_limit_exceeded {
            return Err(process_scan_resource_error());
        }
        Ok(scan.identities)
    }

    fn known_identities(&self) -> Vec<ProcessIdentity> {
        self.known.iter().copied().collect()
    }

    fn reap_adopted_children(&self) -> bool {
        self.reap_adopted
    }
}

#[cfg(any(test, target_os = "macos"))]
#[derive(Clone, Copy, Debug)]
enum MacosProcessEvent {
    Child { pid: libc::pid_t },
    Exit { pid: libc::pid_t },
    Drained,
}

#[cfg(any(test, target_os = "macos"))]
fn reduce_macos_process_event_with<R>(
    _root: ProcessIdentity,
    tracked: &mut HashMap<libc::pid_t, ProcessIdentity>,
    pending: &mut HashMap<libc::pid_t, ProcessIdentity>,
    event: MacosProcessEvent,
    mut read_record: R,
) -> Result<()>
where
    R: FnMut(libc::pid_t) -> Result<Option<ProcessRecord>>,
{
    match event {
        MacosProcessEvent::Child { pid } => {
            let Some(record) = read_record(pid)? else {
                return Ok(());
            };
            let distinct_pending = pending
                .keys()
                .filter(|pending_pid| !tracked.contains_key(pending_pid))
                .count();
            if !tracked.contains_key(&pid)
                && !pending.contains_key(&pid)
                && tracked.len().saturating_add(distinct_pending) >= MAX_PROCESS_CLEANUP_GENERATIONS
            {
                return Err(process_scan_resource_error());
            }
            pending.insert(pid, record.identity);
        }
        MacosProcessEvent::Exit { pid } => {
            pending.remove(&pid);
            if read_record(pid)?.is_none() {
                tracked.remove(&pid);
            }
        }
        MacosProcessEvent::Drained => {
            let candidates: Vec<_> = pending
                .iter()
                .map(|(pid, identity)| (*pid, *identity))
                .collect();
            for (pid, identity) in candidates {
                let current = read_record(pid)?;
                pending.remove(&pid);
                if current.map(|record| record.identity) == Some(identity) {
                    tracked.insert(pid, identity);
                }
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
struct ProcessOwner {
    root: ProcessIdentity,
    queue: Option<File>,
    tracked: HashMap<libc::pid_t, ProcessIdentity>,
    pending: HashMap<libc::pid_t, ProcessIdentity>,
}

#[cfg(target_os = "macos")]
impl ProcessOwner {
    fn new() -> Result<Self> {
        Self::for_root(unsafe { libc::getpid() })
    }

    fn for_root(root_pid: libc::pid_t) -> Result<Self> {
        use std::os::fd::FromRawFd;

        let descriptor = unsafe { libc::kqueue() };
        if descriptor < 0 {
            return Err(process_scan_resource_error());
        }
        let queue = unsafe { File::from_raw_fd(descriptor) };
        let root = process_identity(root_pid)?.ok_or_else(process_scan_resource_error)?;
        let change = libc::kevent {
            ident: root_pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_CLEAR,
            fflags: libc::NOTE_TRACK | libc::NOTE_FORK | libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let changed = unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                &change,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if changed < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOTSUP) {
                // Some macOS kernels expose NOTE_TRACK in libc but reject its
                // registration. Retain the same identity-bound bounded scan
                // used alongside kqueue; every other registration error stays
                // fail-closed.
                return Ok(Self {
                    root,
                    queue: None,
                    tracked: HashMap::new(),
                    pending: HashMap::new(),
                });
            }
            return Err(process_scan_resource_error());
        }
        Ok(Self {
            root,
            queue: Some(queue),
            tracked: HashMap::new(),
            pending: HashMap::new(),
        })
    }

    fn refresh(&mut self, deadline: Instant) -> Result<()> {
        let Some(queue) = &self.queue else {
            return ensure_deadline(deadline);
        };
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        loop {
            ensure_deadline(deadline)?;
            let mut events: [libc::kevent; 64] = unsafe { std::mem::zeroed() };
            let count = unsafe {
                libc::kevent(
                    queue.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    events.len() as libc::c_int,
                    &timeout,
                )
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(process_scan_resource_error());
            }
            if count == 0 {
                reduce_macos_process_event_with(
                    self.root,
                    &mut self.tracked,
                    &mut self.pending,
                    MacosProcessEvent::Drained,
                    |pid| {
                        ensure_deadline(deadline)?;
                        macos_process_record(pid)
                    },
                )?;
                return Ok(());
            }
            for event in &events[..count as usize] {
                ensure_deadline(deadline)?;
                let flags = unsafe { std::ptr::addr_of!(event.fflags).read_unaligned() };
                let pid =
                    unsafe { std::ptr::addr_of!(event.ident).read_unaligned() as libc::pid_t };
                if flags & libc::NOTE_TRACKERR != 0 {
                    return Err(process_scan_resource_error());
                }
                if flags & libc::NOTE_CHILD != 0 {
                    reduce_macos_process_event_with(
                        self.root,
                        &mut self.tracked,
                        &mut self.pending,
                        MacosProcessEvent::Child { pid },
                        macos_process_record,
                    )?;
                }
                if flags & libc::NOTE_EXIT != 0 {
                    reduce_macos_process_event_with(
                        self.root,
                        &mut self.tracked,
                        &mut self.pending,
                        MacosProcessEvent::Exit { pid },
                        macos_process_record,
                    )?;
                }
            }
        }
    }

    fn identities(&mut self, deadline: Instant) -> Result<Vec<ProcessIdentity>> {
        self.refresh(deadline)?;
        let mut identities = HashSet::new();
        let mut stale = Vec::new();
        for identity in self.tracked.values().copied() {
            ensure_deadline(deadline)?;
            if process_identity(identity.pid)? == Some(identity) {
                identities.insert(identity);
            } else {
                stale.push(identity);
            }
        }
        for identity in stale {
            if self.tracked.get(&identity.pid) == Some(&identity) {
                self.tracked.remove(&identity.pid);
            }
        }
        let fallback = macos_fallback_descendants_with(
            self.root,
            deadline,
            process_identity,
            descendant_processes_from_identity,
        )?;
        identities.extend(fallback.identities);
        if identities.len() > PROCESS_SCAN_LIMITS.descendants {
            return Err(process_scan_resource_error());
        }
        for identity in identities.iter().copied() {
            self.tracked.insert(identity.pid, identity);
        }
        Ok(identities.into_iter().collect())
    }

    fn known_identities(&self) -> Vec<ProcessIdentity> {
        self.tracked
            .values()
            .chain(self.pending.values())
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    fn reap_adopted_children(&self) -> bool {
        false
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct ProcessOwner {
    root: libc::pid_t,
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl ProcessOwner {
    fn new() -> Result<Self> {
        Self::for_root(unsafe { libc::getpid() })
    }

    fn for_root(root: libc::pid_t) -> Result<Self> {
        Ok(Self { root })
    }

    fn refresh(&mut self, deadline: Instant) -> Result<()> {
        ensure_deadline(deadline)
    }

    fn identities(&mut self, deadline: Instant) -> Result<Vec<ProcessIdentity>> {
        Ok(descendant_processes(self.root, deadline)?.identities)
    }

    fn known_identities(&self) -> Vec<ProcessIdentity> {
        Vec::new()
    }

    fn reap_adopted_children(&self) -> bool {
        false
    }
}

trait OwnedProcessTracker {
    fn refresh(&mut self, deadline: Instant) -> Result<()> {
        ensure_deadline(deadline)
    }
    fn identities(&mut self, deadline: Instant) -> Result<Vec<ProcessIdentity>>;
    fn known_identities(&self) -> Vec<ProcessIdentity>;
    fn reap_adopted_children(&self) -> bool {
        false
    }
}

impl OwnedProcessTracker for ProcessOwner {
    fn refresh(&mut self, deadline: Instant) -> Result<()> {
        ProcessOwner::refresh(self, deadline)
    }

    fn identities(&mut self, deadline: Instant) -> Result<Vec<ProcessIdentity>> {
        ProcessOwner::identities(self, deadline)
    }

    fn known_identities(&self) -> Vec<ProcessIdentity> {
        ProcessOwner::known_identities(self)
    }

    fn reap_adopted_children(&self) -> bool {
        ProcessOwner::reap_adopted_children(self)
    }
}

trait OwnedProcessControl {
    type Pinned;

    fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>>;
    fn signal(&mut self, process: &Self::Pinned, signal: libc::c_int) -> Result<bool>;
    fn wait_for_exit(&mut self, _process: &Self::Pinned, _deadline: Instant) -> Result<()> {
        Ok(())
    }
}

struct SystemProcessControl;

impl OwnedProcessControl for SystemProcessControl {
    type Pinned = PinnedProcess;

    fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
        pin_process(identity)
    }

    fn signal(&mut self, process: &Self::Pinned, signal: libc::c_int) -> Result<bool> {
        signal_pinned_process(process, signal)
    }

    fn wait_for_exit(&mut self, process: &Self::Pinned, deadline: Instant) -> Result<()> {
        wait_pinned_process_until(process, deadline)
    }
}

fn terminate_owned_identity_until_with<C>(
    control: &mut C,
    identity: ProcessIdentity,
    deadline: Instant,
) -> Result<()>
where
    C: OwnedProcessControl,
{
    ensure_deadline(deadline)?;
    let Some(process) = control.pin(identity)? else {
        return Ok(());
    };
    ensure_deadline(deadline)?;
    if !control.signal(&process, libc::SIGSTOP)? {
        return Ok(());
    }
    if let Err(error) = ensure_deadline(deadline) {
        return match control.signal(&process, libc::SIGCONT) {
            Ok(_) => Err(error),
            Err(resume_error) => Err(resume_error),
        };
    }
    if let Err(error) = control.signal(&process, libc::SIGKILL) {
        return match control.signal(&process, libc::SIGCONT) {
            Ok(_) => Err(error),
            Err(resume_error) => Err(resume_error),
        };
    }
    control.wait_for_exit(&process, deadline)
}

fn terminate_owned_processes<T: OwnedProcessTracker>(
    owner: &mut T,
    child: &mut Child,
) -> Result<()> {
    let deadline = Instant::now()
        .checked_add(PROCESS_CLEANUP_TIMEOUT)
        .unwrap_or_else(Instant::now);
    terminate_owned_processes_until(owner, child, deadline)
}

fn terminate_owned_processes_until<T: OwnedProcessTracker>(
    owner: &mut T,
    child: &mut Child,
    deadline: Instant,
) -> Result<()> {
    terminate_owned_processes_until_with(owner, child, deadline, &mut SystemProcessControl)
}

fn terminate_owned_processes_until_with<T, C>(
    owner: &mut T,
    child: &mut Child,
    deadline: Instant,
    control: &mut C,
) -> Result<()>
where
    T: OwnedProcessTracker,
    C: OwnedProcessControl,
{
    let mut attempted = HashSet::new();
    let mut containment_error = None;
    if let Err(error) = owner.refresh(deadline) {
        containment_error.get_or_insert(error);
    }
    if let Err(error) = child.kill()
        && error.kind() != io::ErrorKind::InvalidInput
    {
        containment_error.get_or_insert_with(process_scan_resource_error);
    }
    'discovery: loop {
        if let Err(error) = ensure_deadline(deadline) {
            containment_error.get_or_insert(error);
            break;
        }
        let mut identities = match owner.identities(deadline) {
            Ok(identities) => identities,
            Err(error) => {
                containment_error.get_or_insert(error);
                Vec::new()
            }
        };
        identities.extend(owner.known_identities());
        let mut added = false;
        for identity in identities {
            if let Err(error) = ensure_deadline(deadline) {
                containment_error.get_or_insert(error);
                break 'discovery;
            }
            if attempted.contains(&identity) {
                continue;
            }
            if attempted.len() >= MAX_PROCESS_CLEANUP_GENERATIONS {
                containment_error.get_or_insert_with(process_scan_resource_error);
                break 'discovery;
            }
            attempted.insert(identity);
            added = true;
            if let Err(error) = terminate_owned_identity_until_with(control, identity, deadline) {
                containment_error.get_or_insert(error);
            }
            if let Err(error) = ensure_deadline(deadline) {
                containment_error.get_or_insert(error);
                break 'discovery;
            }
        }
        if containment_error.is_some() || !added {
            break;
        }
    }
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(None) => {
                containment_error.get_or_insert_with(|| {
                    domain_error(
                        DirtyCheckoutErrorKind::Timeout,
                        "process cleanup exceeded its deadline",
                    )
                });
                break;
            }
            Err(_) => {
                containment_error.get_or_insert_with(process_scan_resource_error);
                break;
            }
        }
    }
    if owner.reap_adopted_children() {
        loop {
            if Instant::now() >= deadline {
                containment_error.get_or_insert_with(|| {
                    domain_error(
                        DirtyCheckoutErrorKind::Timeout,
                        "process cleanup exceeded its deadline",
                    )
                });
                break;
            }
            let reaped = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
            if reaped > 0 {
                continue;
            }
            if reaped < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.raw_os_error() != Some(libc::ECHILD) {
                    containment_error.get_or_insert_with(process_scan_resource_error);
                }
                break;
            }
            if Instant::now() >= deadline {
                containment_error.get_or_insert_with(|| {
                    domain_error(
                        DirtyCheckoutErrorKind::Timeout,
                        "process cleanup exceeded its deadline",
                    )
                });
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if let Some(error) = containment_error {
        Err(error)
    } else {
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct PinnedProcess;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn pin_process(_identity: ProcessIdentity) -> Result<Option<PinnedProcess>> {
    Ok(None)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn signal_pinned_process(_process: &PinnedProcess, _signal: libc::c_int) -> Result<bool> {
    Ok(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn wait_pinned_process_until(_process: &PinnedProcess, _deadline: Instant) -> Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn descendant_processes_with<F>(
    _root: libc::pid_t,
    _deadline: Instant,
    _observe: F,
) -> Result<ProcessScan>
where
    F: FnMut(ProcessIdentity),
{
    Ok(ProcessScan {
        identities: Vec::new(),
        #[cfg(all(test, target_os = "linux"))]
        edge_visits: 0,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn descendant_processes(root: libc::pid_t, deadline: Instant) -> Result<ProcessScan> {
    descendant_processes_with(root, deadline, |_| {})
}

fn read_supervisor_cleanup_proof_with<R: Read>(reader: &mut R) -> bool {
    read_process_supervisor_completion(reader).is_ok_and(|completion| {
        completion.cleanup_complete
            && completion.kind != ProcessSupervisorCompletionKind::InternalFailure
    })
}

fn signal_cleanup_leader_with<F>(pid: libc::pid_t, supervised: bool, mut send: F) -> io::Result<()>
where
    F: FnMut(libc::pid_t, libc::c_int) -> io::Result<()>,
{
    if supervised {
        send(pid, libc::SIGTERM)
    } else {
        send(-pid, libc::SIGSTOP)
    }
}

fn signal_cleanup_leader(pid: libc::pid_t, supervised: bool) -> io::Result<()> {
    signal_cleanup_leader_with(pid, supervised, |target, signal| {
        if unsafe { libc::kill(target, signal) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    })
}

fn should_use_unpinned_cleanup_signals(supervised: bool, leader_reaped: bool) -> bool {
    !supervised || !leader_reaped
}

#[cfg(all(test, target_os = "linux"))]
thread_local! {
    static UNPINNED_CLEANUP_SIGNAL_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(all(test, target_os = "linux"))]
fn record_unpinned_cleanup_signal_attempt() {
    UNPINNED_CLEANUP_SIGNAL_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
}

#[cfg(all(test, target_os = "linux"))]
fn reset_unpinned_cleanup_signal_attempts() {
    UNPINNED_CLEANUP_SIGNAL_ATTEMPTS.with(|attempts| attempts.set(0));
}

#[cfg(all(test, target_os = "linux"))]
fn unpinned_cleanup_signal_attempts() -> usize {
    UNPINNED_CLEANUP_SIGNAL_ATTEMPTS.with(std::cell::Cell::get)
}

fn terminate_child(
    child: &mut Child,
    process_group: libc::pid_t,
    supervised: bool,
    cleanup_proof: &mut Option<std::os::unix::net::UnixStream>,
    fallback_owner: &mut Option<ProcessOwner>,
) {
    let cleanup_started = Instant::now();
    let supervisor_grace = PROCESS_CLEANUP_TIMEOUT
        .checked_add(Duration::from_millis(25))
        .unwrap_or(PROCESS_CLEANUP_TIMEOUT);
    let cleanup_timeout = if supervised {
        supervisor_grace
            .checked_add(PROCESS_CLEANUP_TIMEOUT)
            .unwrap_or(supervisor_grace)
    } else {
        PROCESS_CLEANUP_TIMEOUT
    };
    let cleanup_deadline = cleanup_started
        .checked_add(cleanup_timeout)
        .unwrap_or(cleanup_started);
    let mut leader_reaped = child.try_wait().is_ok_and(|status| status.is_some());
    if should_use_unpinned_cleanup_signals(supervised, leader_reaped) {
        #[cfg(all(test, target_os = "linux"))]
        record_unpinned_cleanup_signal_attempt();
        let _ = signal_cleanup_leader(process_group, supervised);
    }

    if supervised {
        if leader_reaped
            && cleanup_proof
                .as_mut()
                .is_some_and(read_supervisor_cleanup_proof_with)
        {
            return;
        }
        let supervisor_deadline = cleanup_started
            .checked_add(supervisor_grace)
            .unwrap_or(cleanup_started)
            .min(cleanup_deadline);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    leader_reaped = true;
                    if cleanup_proof
                        .as_mut()
                        .is_some_and(read_supervisor_cleanup_proof_with)
                    {
                        return;
                    }
                    break;
                }
                Ok(None) if Instant::now() < supervisor_deadline => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Ok(None) | Err(_) => break,
            }
        }
        if should_use_unpinned_cleanup_signals(supervised, leader_reaped) {
            #[cfg(all(test, target_os = "linux"))]
            record_unpinned_cleanup_signal_attempt();
            unsafe {
                let _ = libc::kill(-process_group, libc::SIGSTOP);
            }
        }
    }

    if let Some(owner) = fallback_owner {
        let _ = terminate_owned_processes_until(owner, child, cleanup_deadline);
    }

    if !supervised {
        let mut known = HashSet::new();
        let mut pending = VecDeque::new();
        let mut control = SystemProcessControl;
        while Instant::now() < cleanup_deadline {
            let mut added = false;
            let mut generation_limit_exceeded = false;
            let scan_result =
                descendant_processes_with(process_group, cleanup_deadline, |identity| {
                    if known.contains(&identity) {
                        return;
                    }
                    if known.len() >= MAX_PROCESS_CLEANUP_GENERATIONS {
                        generation_limit_exceeded = true;
                        return;
                    }
                    known.insert(identity);
                    pending.push_back(identity);
                    added = true;
                });
            while let Some(identity) = pending.pop_front() {
                if Instant::now() >= cleanup_deadline {
                    break;
                }
                let _ =
                    terminate_owned_identity_until_with(&mut control, identity, cleanup_deadline);
            }
            if scan_result.is_err() || generation_limit_exceeded || !added {
                break;
            }
        }
        #[cfg(all(test, target_os = "linux"))]
        record_unpinned_cleanup_signal_attempt();
        unsafe {
            if libc::kill(-process_group, libc::SIGKILL) != 0 {
                let _ = libc::kill(-process_group, libc::SIGCONT);
            }
        }
    }
    let _ = child.kill();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) if Instant::now() < cleanup_deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(None) => break,
        }
    }
}

pub(super) fn run_dirty_snapshot(args: &[String]) -> i32 {
    let requested_format = detect_format(args);
    let mut args = args.to_vec();
    let format = match take_format(&mut args) {
        Ok(format) => format,
        Err(error) => return emit_error("worktree.dirty-snapshot", requested_format, error),
    };
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("Usage: git-cli worktree dirty-snapshot [--format text|json]");
        return 0;
    }
    if !args.is_empty() {
        return emit_error(
            "worktree.dirty-snapshot",
            format,
            CliError::usage(
                "unexpected-argument",
                "worktree dirty-snapshot accepts no positional arguments",
            ),
        );
    }
    match env::current_dir()
        .context("current checkout could not be resolved")
        .and_then(|path| dirty_snapshot_cli(&path))
    {
        Ok(snapshot) => emit_success("worktree.dirty-snapshot", format, &snapshot, || {
            snapshot.snapshot_id.clone()
        }),
        Err(error) => emit_error("worktree.dirty-snapshot", format, adoption_cli_error(error)),
    }
}

pub(super) fn run_adopt_dirty(args: &[String]) -> i32 {
    let requested_format = detect_format(args);
    let parsed = match parse_adopt_args(args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return 0,
        Err(error) => return emit_error("worktree.adopt-dirty", requested_format, error),
    };
    if !feature_enabled() {
        return emit_error(
            "worktree.adopt-dirty",
            parsed.format,
            CliError::data(
                "dirty-checkout-adoption-disabled",
                "dirty-checkout adoption is disabled",
            ),
        );
    }
    let challenge = match read_challenge_descriptor(parsed.challenge_fd) {
        Ok(challenge) => challenge,
        Err(error) => {
            return emit_error(
                "worktree.adopt-dirty",
                parsed.format,
                adoption_cli_error(error),
            );
        }
    };
    let result = env::current_dir()
        .context("current checkout could not be resolved")
        .and_then(|checkout| {
            let state_root = resolve_state_root()?;
            adopt_dirty_cli(&checkout, &state_root, &challenge, &parsed.reason_file)
        });
    match result {
        Ok(receipt) => emit_success("worktree.adopt-dirty", parsed.format, &receipt, || {
            receipt.receipt_id.clone()
        }),
        Err(error) => emit_error(
            "worktree.adopt-dirty",
            parsed.format,
            adoption_cli_error(error),
        ),
    }
}

pub(super) fn run_revoke_dirty(args: &[String]) -> i32 {
    let requested_format = detect_format(args);
    let parsed = match parse_revoke_args(args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return 0,
        Err(error) => return emit_error("worktree.revoke-dirty", requested_format, error),
    };
    let result = env::current_dir()
        .context("current checkout could not be resolved")
        .and_then(|checkout| {
            let state_root = resolve_state_root()?;
            revoke_dirty(&checkout, &state_root, &parsed.receipt)
        });
    match result {
        Ok(()) => emit_success(
            "worktree.revoke-dirty",
            parsed.format,
            &serde_json::json!({ "revoked": true, "receipt_id": parsed.receipt }),
            || "Revoked matching dirty-checkout adoption".to_string(),
        ),
        Err(error) => emit_error(
            "worktree.revoke-dirty",
            parsed.format,
            adoption_cli_error(error),
        ),
    }
}

struct AdoptArgs {
    challenge_fd: libc::c_int,
    reason_file: PathBuf,
    format: OutputFormat,
}

struct RevokeArgs {
    receipt: String,
    format: OutputFormat,
}

fn parse_adopt_args(raw: &[String]) -> std::result::Result<Option<AdoptArgs>, CliError> {
    let mut args = raw.to_vec();
    let format = take_format(&mut args).map_err(redact_adopt_parse_error)?;
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!(
            "Usage: git-cli worktree adopt-dirty --challenge-fd <fd> --reason-file <path> [--format text|json]"
        );
        return Ok(None);
    }
    let mut challenge_fd = None;
    let mut reason_file = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--challenge-fd" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CliError::usage(
                        "missing-challenge-fd",
                        "--challenge-fd requires a descriptor",
                    )
                })?;
                let value = parse_challenge_descriptor(value)?;
                if challenge_fd.replace(value).is_some() {
                    return Err(CliError::usage(
                        "duplicate-challenge-fd",
                        "--challenge-fd may be supplied only once",
                    ));
                }
                index += 2;
            }
            value if value.starts_with("--challenge-fd=") => {
                let value = value.trim_start_matches("--challenge-fd=");
                let value = parse_challenge_descriptor(value)?;
                if challenge_fd.replace(value).is_some() {
                    return Err(CliError::usage(
                        "duplicate-challenge-fd",
                        "--challenge-fd may be supplied only once",
                    ));
                }
                index += 1;
            }
            "--reason-file" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CliError::usage("missing-reason-file", "--reason-file requires a path")
                })?;
                if reason_file.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::usage(
                        "duplicate-reason-file",
                        "--reason-file may be supplied only once",
                    ));
                }
                index += 2;
            }
            value if value.starts_with("--reason-file=") => {
                let value = value.trim_start_matches("--reason-file=");
                if value.is_empty() || reason_file.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::usage(
                        "invalid-reason-file",
                        "--reason-file requires one path",
                    ));
                }
                index += 1;
            }
            _ => {
                return Err(CliError::usage(
                    "unexpected-argument",
                    "unexpected adopt-dirty argument",
                ));
            }
        }
    }
    Ok(Some(AdoptArgs {
        challenge_fd: challenge_fd
            .ok_or_else(|| CliError::usage("missing-challenge-fd", "--challenge-fd is required"))?,
        reason_file: reason_file
            .ok_or_else(|| CliError::usage("missing-reason-file", "--reason-file is required"))?,
        format,
    }))
}

fn parse_challenge_descriptor(value: &str) -> std::result::Result<libc::c_int, CliError> {
    value
        .parse::<libc::c_int>()
        .ok()
        .filter(|descriptor| *descriptor == libc::STDIN_FILENO || *descriptor > libc::STDERR_FILENO)
        .ok_or_else(|| {
            CliError::usage(
                "invalid-challenge-fd",
                "--challenge-fd requires an inherited descriptor",
            )
        })
}

fn challenge_descriptor_error() -> anyhow::Error {
    domain_error(
        DirtyCheckoutErrorKind::InvalidInput,
        "dirty-checkout challenge descriptor is unavailable",
    )
}

fn read_challenge_descriptor(descriptor: libc::c_int) -> Result<String> {
    let challenge = read_challenge_descriptor_from_peer_until(
        descriptor,
        unsafe { libc::getppid() },
        unsafe { libc::geteuid() },
        CHALLENGE_DESCRIPTOR_TIMEOUT,
    );
    if descriptor == libc::STDIN_FILENO && !restore_standard_input() {
        return Err(challenge_descriptor_error());
    }
    challenge
}

fn restore_standard_input() -> bool {
    let null_descriptor = match File::open("/dev/null") {
        Ok(file) => file.into_raw_fd(),
        Err(_) => return false,
    };
    if null_descriptor != libc::STDIN_FILENO {
        let duplicated = unsafe { libc::dup2(null_descriptor, libc::STDIN_FILENO) };
        let _ = unsafe { libc::close(null_descriptor) };
        if duplicated != libc::STDIN_FILENO {
            return false;
        }
    }
    let flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFD) };
    flags >= 0
        && unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } >= 0
}

#[cfg(test)]
fn read_challenge_descriptor_from_peer(
    descriptor: libc::c_int,
    expected_peer_pid: libc::pid_t,
    expected_peer_uid: libc::uid_t,
) -> Result<String> {
    read_challenge_descriptor_from_peer_until(
        descriptor,
        expected_peer_pid,
        expected_peer_uid,
        CHALLENGE_DESCRIPTOR_TIMEOUT,
    )
}

fn read_challenge_descriptor_from_peer_until(
    descriptor: libc::c_int,
    expected_peer_pid: libc::pid_t,
    expected_peer_uid: libc::uid_t,
    timeout: Duration,
) -> Result<String> {
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixStream;

    if descriptor < 0 || matches!(descriptor, libc::STDOUT_FILENO | libc::STDERR_FILENO) {
        return Err(challenge_descriptor_error());
    }
    let descriptor_flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if descriptor_flags < 0 {
        return Err(challenge_descriptor_error());
    }
    let mut stream = unsafe { UnixStream::from_raw_fd(descriptor) };
    if unsafe {
        libc::fcntl(
            descriptor,
            libc::F_SETFD,
            descriptor_flags | libc::FD_CLOEXEC,
        )
    } < 0
    {
        return Err(challenge_descriptor_error());
    }

    let mut socket_type = 0;
    let mut socket_type_length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            descriptor,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut socket_type as *mut libc::c_int).cast(),
            &mut socket_type_length,
        )
    } != 0
        || socket_type_length as usize != std::mem::size_of::<libc::c_int>()
        || socket_type != libc::SOCK_STREAM
        || !challenge_descriptor_peer_matches(descriptor, expected_peer_pid, expected_peer_uid)
    {
        return Err(challenge_descriptor_error());
    }

    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(challenge_descriptor_error)?;
    let mut challenge = [0_u8; CHALLENGE_TOKEN_BYTES + 1];
    let mut challenge_len = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || stream.set_read_timeout(Some(remaining)).is_err() {
            return Err(challenge_descriptor_error());
        }
        match stream.read(&mut challenge[challenge_len..]) {
            Ok(0) => break,
            Ok(read) => {
                challenge_len += read;
                if challenge_len > CHALLENGE_TOKEN_BYTES {
                    return Err(challenge_descriptor_error());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(challenge_descriptor_error()),
        }
    }
    let challenge = &challenge[..challenge_len];
    if challenge.len() != CHALLENGE_TOKEN_BYTES
        || !challenge
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(challenge_descriptor_error());
    }
    String::from_utf8(challenge.to_vec()).map_err(|_| challenge_descriptor_error())
}

fn challenge_descriptor_peer_matches(
    descriptor: libc::c_int,
    expected_peer_pid: libc::pid_t,
    expected_peer_uid: libc::uid_t,
) -> bool {
    #[cfg(target_os = "linux")]
    {
        let mut credentials = std::mem::MaybeUninit::<libc::ucred>::zeroed();
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                descriptor,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                credentials.as_mut_ptr().cast(),
                &mut length,
            )
        } != 0
            || length as usize != std::mem::size_of::<libc::ucred>()
        {
            return false;
        }
        linux_peer_credentials_match(
            unsafe { credentials.assume_init() },
            expected_peer_pid,
            expected_peer_uid,
        )
    }

    #[cfg(target_os = "macos")]
    {
        macos_challenge_descriptor_peer_credentials(descriptor).is_some_and(|credentials| {
            credentials.pid == expected_peer_pid
                && credentials.effective_uid == expected_peer_uid
                && credentials.socket_uid == expected_peer_uid
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (descriptor, expected_peer_pid, expected_peer_uid);
        false
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq, Eq)]
struct MacosChallengeDescriptorPeerCredentials {
    pid: libc::pid_t,
    effective_uid: libc::uid_t,
    socket_uid: libc::uid_t,
}

#[cfg(target_os = "macos")]
fn macos_challenge_descriptor_peer_credentials(
    descriptor: libc::c_int,
) -> Option<MacosChallengeDescriptorPeerCredentials> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AuditToken {
        values: [libc::c_uint; 8],
    }

    #[link(name = "bsm", kind = "dylib")]
    unsafe extern "C" {
        fn audit_token_to_pid(token: AuditToken) -> libc::pid_t;
        fn audit_token_to_euid(token: AuditToken) -> libc::uid_t;
    }

    let mut peer_token = std::mem::MaybeUninit::<AuditToken>::zeroed();
    let mut peer_token_length = std::mem::size_of::<AuditToken>() as libc::socklen_t;
    let mut socket_peer_uid = 0;
    let mut peer_gid = 0;
    if unsafe {
        libc::getsockopt(
            descriptor,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERTOKEN,
            peer_token.as_mut_ptr().cast(),
            &mut peer_token_length,
        )
    } != 0
        || peer_token_length as usize != std::mem::size_of::<AuditToken>()
        || unsafe { libc::getpeereid(descriptor, &mut socket_peer_uid, &mut peer_gid) } != 0
    {
        return None;
    }
    let peer_token = unsafe { peer_token.assume_init() };
    Some(MacosChallengeDescriptorPeerCredentials {
        pid: unsafe { audit_token_to_pid(peer_token) },
        effective_uid: unsafe { audit_token_to_euid(peer_token) },
        socket_uid: socket_peer_uid,
    })
}

fn redact_adopt_parse_error(error: CliError) -> CliError {
    let message = match error.code {
        "missing-format" | "invalid-format" => "--format requires text or json",
        _ => "invalid adopt-dirty arguments",
    };
    CliError::usage(error.code, message)
}

fn parse_revoke_args(raw: &[String]) -> std::result::Result<Option<RevokeArgs>, CliError> {
    parse_revoke_args_inner(raw).map_err(redact_revoke_parse_error)
}

fn parse_revoke_args_inner(raw: &[String]) -> std::result::Result<Option<RevokeArgs>, CliError> {
    let mut args = raw.to_vec();
    let format = take_format(&mut args)?;
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("Usage: git-cli worktree revoke-dirty --receipt <id> [--format text|json]");
        return Ok(None);
    }
    let mut receipt = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--receipt" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CliError::usage("missing-receipt", "--receipt requires an ID")
                })?;
                if receipt.replace(value.clone()).is_some() {
                    return Err(CliError::usage(
                        "duplicate-receipt",
                        "--receipt may be supplied only once",
                    ));
                }
                index += 2;
            }
            value if value.starts_with("--receipt=") => {
                let value = value.trim_start_matches("--receipt=");
                if value.is_empty() || receipt.replace(value.to_string()).is_some() {
                    return Err(CliError::usage(
                        "invalid-receipt",
                        "--receipt requires one ID",
                    ));
                }
                index += 1;
            }
            value => {
                return Err(CliError::usage(
                    "unexpected-argument",
                    format!("unexpected revoke-dirty argument: {value}"),
                ));
            }
        }
    }
    Ok(Some(RevokeArgs {
        receipt: receipt
            .ok_or_else(|| CliError::usage("missing-receipt", "--receipt is required"))?,
        format,
    }))
}

fn redact_revoke_parse_error(error: CliError) -> CliError {
    let message = match error.code {
        "missing-format" | "invalid-format" => "--format requires text or json",
        _ => "invalid revoke-dirty arguments",
    };
    CliError::usage(error.code, message)
}

fn adoption_cli_error(error: anyhow::Error) -> CliError {
    if let Some(domain) = error.downcast_ref::<DirtyCheckoutError>() {
        return if domain.kind.exit_code() == exit::RUNTIME {
            CliError::runtime(domain.code(), domain.to_string())
        } else {
            CliError::data(domain.code(), domain.to_string())
        };
    }
    CliError::runtime("dirty-checkout-command-failed", error.to_string())
}

fn feature_enabled() -> bool {
    env::var_os("AGENT_RUNTIME_DIRTY_CHECKOUT_ADOPTION").as_deref() == Some(OsStr::new("1"))
}

fn resolve_state_root() -> Result<PathBuf> {
    nils_common::worktree_lifecycle::state_home()
        .map_err(|_| anyhow::anyhow!("runtime state root is unavailable"))
}

#[cfg(test)]
mod tests;
