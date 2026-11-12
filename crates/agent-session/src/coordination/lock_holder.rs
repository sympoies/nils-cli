//! Secret-free holder descriptor for the coordination registry lock.
//!
//! While a caller holds `registry.lock` it publishes a sidecar next to the
//! lock with the operation class, its process id and the acquisition time. A
//! caller that times out waiting for the lock reads it to say who held the
//! lock and for how long. The descriptor never carries session ids, tokens,
//! paths or message content, and is advisory: the lock itself stays the only
//! source of mutual exclusion.
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use nils_common::fs::write_atomic;
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

/// The sidecar this holder published; removed on drop. The owner drops it
/// before releasing the lock so a successor's descriptor is never removed.
pub(super) struct PublishedHolder {
    path: PathBuf,
}

impl PublishedHolder {
    /// Best effort: a failed write leaves the lock usable and the timeout
    /// diagnosis without a holder.
    pub(super) fn publish(root: &Path, class: HolderClass) -> Option<Self> {
        let descriptor = Descriptor {
            operation: class.as_str().to_string(),
            pid: std::process::id(),
            acquired_at_epoch_ms: now_epoch_ms(),
        };
        let path = root.join(HOLDER_FILE);
        let bytes = serde_json::to_vec(&descriptor).ok()?;
        write_atomic(&path, &bytes, SECRET_FILE_MODE).ok()?;
        Some(Self { path })
    }
}

impl Drop for PublishedHolder {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn process_is_gone(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    if pid == 0 {
        return true;
    }
    // SAFETY: signal 0 only probes for the process.
    let alive = unsafe { libc::kill(pid, 0) } == 0
        || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    !alive
}

fn describe(root: &Path) -> Option<Value> {
    let bytes = read_private_file(&root.join(HOLDER_FILE), MAX_DESCRIPTOR_BYTES).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
    HolderClass::ALL
        .iter()
        .find(|class| class.as_str() == descriptor.operation)?;
    if process_is_gone(descriptor.pid) {
        // The holder crashed or was killed without retiring its descriptor, so
        // nothing in it describes the current holder.
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
