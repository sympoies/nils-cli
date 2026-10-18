//! Bounded receipt retention with a durable rejection fence for evicted UUIDv7 IDs.
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde_json::Value;

use super::super::{read_private_file, store_corrupt, store_unavailable};
use super::operation::{MAX_OPERATIONS, MAX_RECEIPT_BYTES, PREFIX};
use crate::CliError;

const WATERMARK: &str = "console-start-expiry-watermark";
const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

pub(super) fn timestamp_ms(value: &str) -> Option<u64> {
    let id = uuid::Uuid::parse_str(value).ok()?;
    (id.get_version_num() == 7 && id.to_string() == value).then(|| (id.as_u128() >> 80) as u64)
}

fn expired() -> CliError {
    CliError::usage(
        "console-start-operation-expired",
        "operation is expired or was evicted; inspect Console before submitting a new operation",
        None,
    )
}

fn watermark(root: &Path) -> Result<u64, CliError> {
    match read_private_file(&root.join(WATERMARK), 64) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| store_corrupt()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(_) => Err(store_unavailable()),
    }
}

/// Caller holds the short operation-store lock. Existing receipts are read before this admission.
pub(super) fn admit(root: &Path, started_ms: u64) -> Result<(), CliError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| store_unavailable())?
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let oldest = now.saturating_sub(RETENTION_MS);
    let cutoff = watermark(root)?;
    if started_ms <= cutoff || started_ms <= oldest || started_ms > now.saturating_add(60_000) {
        return Err(expired());
    }
    let entries = fs::read_dir(root)
        .map_err(|_| store_unavailable())?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(PREFIX) && name.ends_with(".json")
        })
        .take(MAX_OPERATIONS + 1)
        .collect::<Vec<_>>();
    if entries.len() < MAX_OPERATIONS {
        return Ok(());
    }
    if entries.len() > MAX_OPERATIONS {
        return Err(store_corrupt());
    }
    let mut removable = Vec::new();
    for entry in entries {
        let bytes =
            read_private_file(&entry.path(), MAX_RECEIPT_BYTES).map_err(|_| store_unavailable())?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| store_corrupt())?;
        let ms = value["operation_started_at_ms"]
            .as_u64()
            .ok_or_else(store_corrupt)?;
        if ms <= oldest || matches!(value["state"].as_str(), Some("created" | "not_created")) {
            removable.push((ms, entry.path()));
        }
    }
    removable.sort_by_key(|(ms, _)| *ms);
    removable.truncate(MAX_OPERATIONS / 4);
    let Some(newest_removed) = removable.iter().map(|(ms, _)| *ms).max() else {
        return Err(CliError::runtime(
            "console-start-operation-capacity",
            "recent ambiguous starts occupy the receipt store; reconcile them or wait for seven-day expiry",
            None,
        ));
    };
    let cutoff = cutoff.max(newest_removed);
    let bytes = serde_json::to_vec(&cutoff).map_err(|_| store_unavailable())?;
    write_atomic(&root.join(WATERMARK), &bytes, SECRET_FILE_MODE)
        .map_err(|_| store_unavailable())?;
    // The rejection fence must survive before any receipt can disappear.
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| store_unavailable())?;
    for (_, path) in removable {
        fs::remove_file(path).map_err(|_| store_unavailable())?;
    }
    if started_ms <= cutoff {
        return Err(expired());
    }
    Ok(())
}
