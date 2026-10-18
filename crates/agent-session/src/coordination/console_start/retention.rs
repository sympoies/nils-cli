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
    let mut cutoff = watermark(root)?;
    if started_ms <= cutoff || started_ms <= oldest || started_ms > now.saturating_add(60_000) {
        return Err(expired());
    }
    loop {
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
        let mut removable = Vec::new();
        for entry in entries {
            let bytes = read_private_file(&entry.path(), MAX_RECEIPT_BYTES)
                .map_err(|_| store_unavailable())?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| store_corrupt())?;
            let ms = value["operation_started_at_ms"]
                .as_u64()
                .ok_or_else(store_corrupt)?;
            removable.push((ms, entry.path()));
        }
        removable.sort_by_key(|(ms, _)| *ms);
        removable.truncate(MAX_OPERATIONS / 4);
        let newest_removed = removable
            .iter()
            .map(|(ms, _)| *ms)
            .max()
            .ok_or_else(store_corrupt)?;
        cutoff = cutoff.max(newest_removed);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn full_store_recovers(count: usize, state: &str) {
        let root = tempfile::tempdir().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let started = now - 10_000;
        for index in 0..count {
            write_atomic(
                &root.path().join(format!("{PREFIX}fixture-{index}.json")),
                &serde_json::to_vec(
                    &json!({"state":state,"operation_started_at_ms":started + index as u64}),
                )
                .unwrap(),
                SECRET_FILE_MODE,
            )
            .unwrap();
        }
        admit(root.path(), now).expect("capacity maintenance must admit a fresh operation");
        let retained = fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
            .count();
        assert!(
            retained < MAX_OPERATIONS,
            "maintenance must leave room for admission"
        );
        assert!(watermark(root.path()).unwrap() >= started);
        let error =
            admit(root.path(), started).expect_err("an evicted operation must never be readmitted");
        assert_eq!(error.0.code, "console-start-operation-expired");
    }

    #[test]
    fn over_capacity_store_reclaims_space_instead_of_becoming_corrupt() {
        full_store_recovers(MAX_OPERATIONS + MAX_OPERATIONS / 4 + 1, "created");
    }

    #[test]
    fn recent_ambiguous_store_reclaims_space_without_readmitting_evicted_operations() {
        full_store_recovers(MAX_OPERATIONS, "ambiguous");
    }
}
