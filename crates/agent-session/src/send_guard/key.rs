//! Durable claims are scoped to the stable session, independent of its runtime.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{CliContext, CliError, SessionRecord, session_dir};

fn path(context: &CliContext, record: &SessionRecord, key: &str) -> Result<PathBuf, CliError> {
    if key.is_empty() || key.len() > 256 {
        return Err(CliError::usage(
            "send-invalid-idempotency-key",
            "idempotency key must contain 1 to 256 bytes",
            None,
        ));
    }
    let mut digest = Sha256::new();
    digest.update(b"agent-session.send-key.v1\0");
    digest.update(key.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = digest.finalize();
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes.as_slice() {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(session_dir(context, &record.id).join(format!("send-key-{hex}.json")))
}

fn used(record: &SessionRecord) -> CliError {
    CliError::runtime(
        "send-idempotency-key-used",
        "idempotency key was already claimed for this session",
        Some(json!({"id": record.id, "guard": "idempotency_key", "atomic": false})),
    )
}

pub(super) fn check(
    context: &CliContext,
    record: &SessionRecord,
    key: Option<&str>,
) -> Result<(), CliError> {
    let Some(key) = key else {
        return Ok(());
    };
    match path(context, record, key)?.try_exists() {
        Ok(true) => Err(used(record)),
        Ok(false) => Ok(()),
        Err(err) => Err(persistence_error(err)),
    }
}

pub(super) fn claim(
    context: &CliContext,
    record: &SessionRecord,
    key: Option<&str>,
) -> Result<(), CliError> {
    let Some(key) = key else {
        return Ok(());
    };
    let path = path(context, record, key)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| {
            if err.kind() == io::ErrorKind::AlreadyExists {
                used(record)
            } else {
                persistence_error(err)
            }
        })?;
    // A partial or failed claim remains consumed. Never retry an uncertain input.
    let receipt = json!({"schema_version": "agent-session.send-claim.v1", "id": record.id,
        "runtime_id": record.runtime.as_ref().map(|runtime| &runtime.launch_id)});
    file.write_all(receipt.to_string().as_bytes())
        .map_err(persistence_error)?;
    file.sync_all().map_err(persistence_error)?;
    File::open(session_dir(context, &record.id))
        .and_then(|dir| dir.sync_all())
        .map_err(persistence_error)
}

fn persistence_error(err: io::Error) -> CliError {
    CliError::runtime(
        "send-idempotency-persist-failed",
        format!("could not persist the delivery claim: {err}"),
        None,
    )
}
