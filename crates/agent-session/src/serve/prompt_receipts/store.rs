//! Immutable, durable pre-dispatch claim plus atomic terminal result.
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Receipt, ResultRecord};
use crate::{CliContext, CliError, session_dir};

const MAX_BINDING_SESSIONS: usize = 4096;
const MAX_BINDING_RESULTS: usize = 20_000;
const PROMPT_BINDING_SCAN_BUDGET: std::time::Duration = std::time::Duration::from_millis(250);

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

pub(super) fn history_bindings(
    state_dir: &Path,
) -> HashMap<String, HashMap<String, crate::provider_history::PromptRequestBinding>> {
    let mut bindings = HashMap::new();
    let mut ambiguous = HashSet::new();
    let Ok(sessions) = fs::read_dir(state_dir.join("sessions")) else {
        return bindings;
    };
    let mut scanned_sessions = 0usize;
    let mut scanned_results = 0usize;
    let deadline = std::time::Instant::now() + PROMPT_BINDING_SCAN_BUDGET;
    for session in sessions.flatten() {
        scanned_sessions += 1;
        if scanned_sessions > MAX_BINDING_SESSIONS || std::time::Instant::now() >= deadline {
            return HashMap::new();
        }
        let Ok(kind) = session.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let Ok(directory_session_id) = session.file_name().into_string() else {
            continue;
        };
        if !valid_binding_session_id(&directory_session_id) {
            continue;
        }
        let Ok(results) = fs::read_dir(session.path()) else {
            continue;
        };
        for result in results.flatten() {
            if scanned_results >= MAX_BINDING_RESULTS || std::time::Instant::now() >= deadline {
                return HashMap::new();
            }
            let name = result.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("prompt-receipt-") || !name.ends_with(".result") {
                continue;
            }
            scanned_results += 1;
            if !result.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let Ok(bytes) = read(&result.path()) else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<ResultRecord>(&bytes) else {
                continue;
            };
            let Some(turn_id) = record
                .history_turn_id
                .filter(|id| valid_history_turn_id(id))
            else {
                continue;
            };
            let Some(provider_session_id) = record
                .history_session_id
                .filter(|id| valid_provider_session_id(id))
            else {
                continue;
            };
            if record.receipt.outcome != super::Outcome::Submitted
                || !super::valid_request_id(&record.receipt.request_id)
                || !super::valid_incarnation(&record.receipt.session_incarnation)
                || !valid_binding_session_id(&record.receipt.session_id)
                || record.receipt.session_id != directory_session_id
            {
                continue;
            }
            let ambiguity_key = (provider_session_id.clone(), turn_id.clone());
            if ambiguous.contains(&ambiguity_key) {
                continue;
            }
            let binding = crate::provider_history::PromptRequestBinding {
                request_id: record.receipt.request_id,
                session_id: record.receipt.session_id,
                session_incarnation: record.receipt.session_incarnation,
            };
            let session_bindings = bindings.entry(provider_session_id.clone()).or_default();
            if session_bindings.insert(turn_id.clone(), binding).is_some() {
                session_bindings.remove(&turn_id);
                ambiguous.insert(ambiguity_key);
            }
        }
    }
    bindings
}

fn valid_history_turn_id(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 256
        && !value.contains(['/', '\\'])
        && !value.contains("://")
}

fn valid_provider_session_id(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 256
        && !value.contains(['/', '\\'])
        && !value.contains("://")
}

fn valid_binding_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(super) fn digest(value: &impl Serialize) -> String {
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

pub(super) fn confirmation_is_current(context: &CliContext, result: &ResultRecord) -> bool {
    let Some(proof) = result.native_confirmation.as_ref() else {
        return false;
    };
    let claim = read(&path(context, &result.receipt))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ClaimRecord>(&bytes).ok());
    let Some(claim) = claim else { return false };
    claim.fingerprint == proof.fingerprint
        && claim.receipt.request_id == result.receipt.request_id
        && claim.receipt.session_id == result.receipt.session_id
        && claim.receipt.session_incarnation == result.receipt.session_incarnation
        && lookup(context, &result.receipt)
            .ok()
            .flatten()
            .is_some_and(|current| {
                current.receipt.outcome == super::Outcome::Unknown
                    && current.native_confirmation.as_ref() == Some(proof)
            })
}

pub(super) fn publish_confirmed(
    context: &CliContext,
    result: &ResultRecord,
) -> Result<Option<ResultRecord>, CliError> {
    if !confirmation_is_current(context, result) {
        return Ok(None);
    }
    let mut confirmed = result.clone();
    confirmed.receipt.outcome = super::Outcome::Submitted;
    confirmed.receipt.code = None;
    confirmed.receipt.confirmation_diagnostic = None;
    confirmed.status = 200;
    confirmed.queued = false;
    confirmed.history_turn_id = confirmed
        .native_confirmation
        .as_ref()
        .map(|proof| proof.turn_id.clone());
    confirmed.history_session_id = confirmed
        .native_confirmation
        .as_ref()
        .and_then(|proof| proof.history_session_id.clone());
    confirmed.native_confirmation = None;
    finish(
        Claim {
            receipt: confirmed.receipt.clone(),
            path: path(context, &confirmed.receipt),
        },
        &confirmed,
    )?;
    Ok(Some(confirmed))
}

#[cfg(test)]
mod tests;
