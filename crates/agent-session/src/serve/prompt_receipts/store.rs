//! Immutable, durable pre-dispatch claim plus atomic terminal result.
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Receipt, ResultRecord};
use crate::{CliContext, CliError, session_dir};

#[derive(Serialize, Deserialize)]
struct ClaimRecord {
    fingerprint: String,
    receipt: Receipt,
}

pub(super) struct Claim {
    pub(super) receipt: Receipt,
    path: PathBuf,
}

pub(super) enum ClaimResult {
    New(Claim),
    Existing(ResultRecord),
}

pub(super) fn fingerprint(text: &str, require_idle: bool) -> String {
    digest(&(text, require_idle))
}

fn digest(value: &impl Serialize) -> String {
    let bytes = Sha256::digest(serde_json::to_vec(value).expect("fixed serializable tuple"));
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("write to string");
    }
    encoded
}

fn path(context: &CliContext, receipt: &Receipt) -> PathBuf {
    let key = digest(&(&receipt.session_incarnation, &receipt.request_id));
    session_dir(context, &receipt.session_id).join(format!("prompt-receipt-{key}.json"))
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut bytes = Vec::new();
    file.take(16 * 1024).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn persist_error(_: impl std::fmt::Display) -> CliError {
    CliError::runtime(
        "prompt-receipt-storage-unavailable",
        "prompt receipt storage is unavailable; do not replay an uncertain attempt",
        None,
    )
}

fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().expect("session directory"))?.sync_all()
}

pub(super) fn claim(
    context: &CliContext,
    receipt: Receipt,
    fingerprint: &str,
) -> Result<ClaimResult, CliError> {
    let path = path(context, &receipt);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            // A partial claim is conservatively unknown. It cannot be reused,
            // even if the original writer failed before reaching dispatch.
            if let Ok(bytes) = read(&path)
                && let Ok(claim) = serde_json::from_slice::<ClaimRecord>(&bytes)
                && claim.fingerprint != fingerprint
            {
                return Err(CliError::data(
                    "request-id-conflict",
                    "request_id was already used with another payload",
                    None,
                ));
            }
            return Ok(ClaimResult::Existing(
                lookup(context, &receipt)?.unwrap_or_else(|| ResultRecord::unknown(receipt)),
            ));
        }
        Err(error) => return Err(persist_error(error)),
    };
    let record = ClaimRecord {
        fingerprint: fingerprint.to_string(),
        receipt: receipt.clone(),
    };
    let bytes = serde_json::to_vec(&record).map_err(persist_error)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| sync_parent(&path))
        .map_err(persist_error)?;
    Ok(ClaimResult::New(Claim { receipt, path }))
}

pub(super) fn lookup(
    context: &CliContext,
    receipt: &Receipt,
) -> Result<Option<ResultRecord>, CliError> {
    let claim_path = path(context, receipt);
    // Check the claim even if it is partial. Only absence proves not-found.
    match fs::symlink_metadata(&claim_path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(persist_error(error)),
    }
    let result = read(&claim_path.with_extension("result"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ResultRecord>(&bytes).ok())
        .filter(|result| {
            result.receipt.request_id == receipt.request_id
                && result.receipt.session_id == receipt.session_id
                && result.receipt.session_incarnation == receipt.session_incarnation
        })
        .unwrap_or_else(|| ResultRecord::unknown(receipt.clone()));
    Ok(Some(result))
}

pub(super) fn finish(claim: Claim, result: &ResultRecord) -> Result<(), CliError> {
    let destination = claim.path.with_extension("result");
    let temporary = claim
        .path
        .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let write = || -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(result)?)?;
        file.sync_all()?;
        fs::rename(&temporary, &destination)?;
        sync_parent(&destination)
    };
    let result = write().map_err(persist_error);
    let _ = fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests;
