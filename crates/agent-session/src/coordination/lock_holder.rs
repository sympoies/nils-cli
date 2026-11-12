//! Secret-free holder descriptor for the coordination registry lock.
//!
//! While a caller holds `registry.lock` it publishes a sidecar next to the
//! lock with the operation class, its process id and the acquisition time. A
//! caller that times out waiting for the lock reads it to say who held the
//! lock and for how long. The descriptor never carries session ids, tokens,
//! paths or message content, and is advisory: the lock itself stays the only
//! source of mutual exclusion.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{SECRET_FILE_MODE, read_private_file};
use crate::CliError;

pub(super) const HOLDER_FILE: &str = "registry.holder";
const MAX_DESCRIPTOR_BYTES: u64 = 512;

/// Closed set of operation classes; the descriptor never names anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HolderClass {
    FullMaintenance,
    Observational,
    Lifecycle,
    Reminder,
}

impl HolderClass {
    const ALL: [Self; 4] = [
        Self::FullMaintenance,
        Self::Observational,
        Self::Lifecycle,
        Self::Reminder,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::FullMaintenance => "full-maintenance",
            Self::Observational => "observational",
            Self::Lifecycle => "lifecycle",
            Self::Reminder => "reminder",
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    operation: String,
    pid: u32,
    // Distinguishes the holder from an unrelated process that reuses its pid.
    process_start: Option<u64>,
    acquired_at_epoch_ms: u64,
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// The sidecar this holder wrote; cleared on drop. The owner drops it before
/// releasing the lock so a successor's descriptor is never cleared.
///
/// The write is advisory and runs while the lock is held, so it stays cheap:
/// one sidecar is rewritten in place, with no fsync and no rename. A reader
/// racing the rewrite sees an unparsable file and reports no holder.
pub(super) struct PublishedHolder {
    file: File,
}

impl PublishedHolder {
    /// Best effort: a failed write leaves the lock usable and the timeout
    /// diagnosis without a holder.
    pub(super) fn publish(root: &Path, class: HolderClass) -> Option<Self> {
        let descriptor = Descriptor {
            operation: class.as_str().to_string(),
            pid: std::process::id(),
            process_start: own_process_start(),
            acquired_at_epoch_ms: now_epoch_ms(),
        };
        let bytes = serde_json::to_vec(&descriptor).ok()?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(SECRET_FILE_MODE)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join(HOLDER_FILE))
            .ok()?;
        file.write_all(&bytes).ok()?;
        Some(Self { file })
    }
}

impl Drop for PublishedHolder {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
    }
}

/// This process's start time never changes, so it is read once per process.
fn own_process_start() -> Option<u64> {
    static OWN: OnceLock<(u32, Option<u64>)> = OnceLock::new();
    let pid = std::process::id();
    let (cached_pid, start) = OWN.get_or_init(|| (pid, process_start(pid)));
    if *cached_pid == pid {
        *start
    } else {
        process_start(pid)
    }
}

/// Kernel start time of a live, non-zombie process (Linux `/proc` identity,
/// as used by `process_observation`); `None` when it cannot be confirmed.
#[cfg(target_os = "linux")]
pub(super) fn process_start(pid: u32) -> Option<u64> {
    let pid = libc::pid_t::try_from(pid).ok()?;
    match crate::read_linux_process_identity(pid) {
        Ok(Some(identity)) if !identity.zombie => Some(identity.start_time),
        _ => None,
    }
}

// No portable process start time exists here, so ownership is never
// confirmed and a descriptor is always reported stale.
#[cfg(not(target_os = "linux"))]
pub(super) fn process_start(_pid: u32) -> Option<u64> {
    None
}

fn describe(root: &Path) -> Option<Value> {
    let bytes = read_private_file(&root.join(HOLDER_FILE), MAX_DESCRIPTOR_BYTES).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
    HolderClass::ALL
        .iter()
        .find(|class| class.as_str() == descriptor.operation)?;
    let owner_confirmed = descriptor.process_start.is_some()
        && process_start(descriptor.pid) == descriptor.process_start;
    if !owner_confirmed {
        // The holder exited without retiring its descriptor (or its pid was
        // reused), so nothing in it describes the current holder.
        return Some(json!({ "state": "stale" }));
    }
    Some(json!({
        "state": "active",
        "operation": descriptor.operation,
        "pid": descriptor.pid,
        "held_ms": now_epoch_ms().saturating_sub(descriptor.acquired_at_epoch_ms),
    }))
}

/// Adds `details.holder` to a registry lock timeout; other errors and
/// timeouts without a readable descriptor are returned unchanged.
pub(super) fn annotate_timeout(root: &Path, mut error: CliError) -> CliError {
    if error.code() != "coordination-lock-timeout" {
        return error;
    }
    if let Some(holder) = describe(root) {
        error.details_mut().get_or_insert_with(|| json!({}))["holder"] = holder;
    }
    error
}
