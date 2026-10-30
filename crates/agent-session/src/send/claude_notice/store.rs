//! Finite, append-only notice claims and row attestations; no text replay storage.
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{Binding, PROVIDER_VERSION, runtime, sha256};
use crate::provider_prompt::native_notice::{Proof, Stamp};
use crate::{CliContext, CliError};

const SCHEMA: &str = "agent-session.claude-notice-ledger.v1";
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORD: usize = 4096;
const MAX_CLAIMS: usize = MAX_BYTES / (2 * MAX_RECORD);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Claim {
    schema_version: String,
    pub(crate) op: String,
    pub(crate) binding: Binding,
    #[serde(skip_serializing_if = "Option::is_none")]
    caller_key_sha256: Option<String>,
    pub(crate) history_id: String,
    pub(crate) envelope_bytes: usize,
    pub(crate) envelope_sha256: String,
    original_budget_ms: u64,
    pub(crate) baseline: Stamp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Submitted,
    Dropped,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Terminal {
    schema_version: String,
    pub(crate) op: String,
    claim_sha256: String,
    state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    row_offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    row_payload_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    envelope_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confirmation_elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<Reason>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    Guard,
    Drop,
    ApiResultMismatch,
    Timeout,
    IdentityChanged,
    Storage,
    AmbiguousHistory,
    HistoryUnavailable,
}

#[derive(Clone)]
pub(crate) struct Confirmed {
    pub(crate) claim: Claim,
    pub(crate) native_uuid: String,
    pub(crate) row_offset: u64,
    pub(crate) row_payload_sha256: String,
    pub(crate) timestamp: Option<String>,
}

pub(crate) struct Token {
    pub(crate) claim: Claim,
    hash: String,
}

struct Ledger {
    file: File,
    parent: PathBuf,
}

impl Drop for Ledger {
    fn drop(&mut self) {
        // SAFETY: this file descriptor is owned by the still-live File.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "notice ledger unavailable")
}
fn error(_: impl std::fmt::Display) -> CliError {
    runtime::unavailable()
}

pub(crate) fn hash_valid(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn id_valid(id: &str, max: usize) -> bool {
    !id.is_empty()
        && id.len() <= max
        && !id.contains(['/', '\\'])
        && !id.chars().any(char::is_control)
}

fn binding_valid(binding: &Binding) -> bool {
    binding.provider == "claude"
        && binding.provider_version == PROVIDER_VERSION
        && id_valid(&binding.launch.session_id, 128)
        && id_valid(&binding.launch.incarnation, 128)
        && binding.launch.generation > 0
        && id_valid(&binding.provider_session_id, 256)
        && binding
            .launch
            .profile
            .as_deref()
            .is_none_or(|profile| id_valid(profile, 256))
        && hash_valid(&binding.module_sha256)
}

impl Claim {
    pub(crate) fn new(
        op: String,
        binding: Binding,
        text: &str,
        key: Option<&str>,
        baseline: Stamp,
        budget: Duration,
    ) -> Self {
        Self {
            schema_version: SCHEMA.into(),
            op,
            history_id: crate::provider_history::stable_history_id(
                &binding.provider,
                binding.launch.profile.as_deref(),
                &binding.provider_session_id,
            ),
            binding,
            caller_key_sha256: key.map(|key| sha256(key.as_bytes())),
            envelope_bytes: text.len(),
            envelope_sha256: sha256(text.as_bytes()),
            original_budget_ms: budget.as_millis() as u64,
            baseline,
        }
    }

    fn valid(&self) -> bool {
        self.schema_version == SCHEMA
            && uuid::Uuid::parse_str(&self.op).is_ok()
            && binding_valid(&self.binding)
            && self.envelope_bytes > 0
            && self.envelope_bytes <= crate::provider_prompt::MAX_PROVIDER_PROMPT_BYTES
            && hash_valid(&self.envelope_sha256)
            && hash_valid(&self.baseline.continuity_sha256)
            && self.caller_key_sha256.as_deref().is_none_or(hash_valid)
            && self.original_budget_ms > 0
            && self.original_budget_ms <= 15_000
            && self.history_id
                == crate::provider_history::stable_history_id(
                    &self.binding.provider,
                    self.binding.launch.profile.as_deref(),
                    &self.binding.provider_session_id,
                )
    }
}

impl Terminal {
    fn confirmed(&self, claim: &Claim) -> Option<Confirmed> {
        if self.state != State::Submitted
            || self.reason.is_some()
            || self.envelope_sha256.as_deref()? != claim.envelope_sha256
            || self.confirmation_elapsed_ms? > claim.original_budget_ms
        {
            return None;
        }
        let native_uuid = self.native_uuid.as_ref()?;
        uuid::Uuid::parse_str(native_uuid).ok()?;
        let offset = self.row_offset?;
        let hash = self.row_payload_sha256.as_ref()?;
        if offset < claim.baseline.eof
            || !hash_valid(hash)
            || self
                .timestamp
                .as_ref()
                .is_some_and(|timestamp| timestamp.len() > 64)
        {
            return None;
        }
        Some(Confirmed {
            claim: claim.clone(),
            native_uuid: native_uuid.clone(),
            row_offset: offset,
            row_payload_sha256: hash.clone(),
            timestamp: self.timestamp.clone(),
        })
    }

    fn valid(&self, claim: &Claim) -> bool {
        if self.schema_version != SCHEMA || self.op != claim.op {
            return false;
        }
        match self.state {
            State::Submitted => self.confirmed(claim).is_some(),
            State::Dropped | State::Unknown => {
                self.reason.is_some()
                    && self.native_uuid.is_none()
                    && self.row_offset.is_none()
                    && self.row_payload_sha256.is_none()
                    && self.envelope_sha256.is_none()
                    && self.timestamp.is_none()
                    && self.confirmation_elapsed_ms.is_none()
            }
        }
    }
}

struct Scan {
    claims: HashMap<String, (Claim, String)>,
    terminals: HashMap<String, Terminal>,
}

impl Ledger {
    fn open(state: &Path, write: bool, deadline: Instant) -> io::Result<Option<Self>> {
        let parent = state.join("history/claude-notices");
        if write {
            fs::create_dir_all(state.join("history"))?;
            runtime::private_dir(&parent).map_err(|_| invalid())?;
        } else if !parent.exists() {
            return Ok(None);
        }
        let dir = fs::symlink_metadata(&parent)?;
        // SAFETY: geteuid has no preconditions.
        if !dir.is_dir() || dir.uid() != unsafe { libc::geteuid() } || dir.mode() & 0o777 != 0o700 {
            return Err(invalid());
        }
        let path = parent.join("ledger.jsonl");
        let mut options = OpenOptions::new();
        options.read(true).custom_flags(libc::O_NOFOLLOW);
        if write {
            options.append(true).create(true).mode(0o600);
        }
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if !write && error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let meta = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o777 != 0o600
            || meta.len() > MAX_BYTES as u64
        {
            return Err(invalid());
        }
        let operation = if write { libc::LOCK_EX } else { libc::LOCK_SH };
        loop {
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            // SAFETY: file owns the descriptor for the entire lock lifetime.
            if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(error);
            }
            std::thread::sleep(
                Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        let after = fs::symlink_metadata(path)?;
        if (meta.dev(), meta.ino()) != (after.dev(), after.ino()) {
            return Err(invalid());
        }
        Ok(Some(Self { file, parent }))
    }

    fn scan(&mut self, deadline: Instant) -> io::Result<Scan> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        (&self.file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES || bytes.last().is_some_and(|byte| *byte != b'\n') {
            return Err(invalid());
        }
        let mut claims = HashMap::new();
        let mut terminals = HashMap::new();
        for (index, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
            if index >= MAX_CLAIMS * 2 || line.len() > MAX_RECORD || Instant::now() >= deadline {
                return Err(invalid());
            }
            let payload = &line[..line.len() - 1];
            let mut value: serde_json::Value = serde_json::from_slice(payload)?;
            let kind = value
                .as_object_mut()
                .ok_or_else(invalid)?
                .remove("kind")
                .ok_or_else(invalid)?;
            match kind.as_str() {
                Some("claim") => {
                    let claim: Claim = serde_json::from_value(value)?;
                    if !claim.valid()
                        || claims.len() >= MAX_CLAIMS
                        || claims.contains_key(&claim.op)
                    {
                        return Err(invalid());
                    }
                    claims.insert(claim.op.clone(), (claim, sha256(payload)));
                }
                Some("terminal") => {
                    let terminal: Terminal = serde_json::from_value(value)?;
                    let (claim, hash) = claims.get(&terminal.op).ok_or_else(invalid)?;
                    if terminal.claim_sha256 != *hash
                        || !terminal.valid(claim)
                        || terminals.insert(terminal.op.clone(), terminal).is_some()
                    {
                        return Err(invalid());
                    }
                }
                _ => return Err(invalid()),
            }
        }
        Ok(Scan { claims, terminals })
    }

    fn append<T: Serialize>(
        &mut self,
        kind: &str,
        record: &T,
        deadline: Instant,
    ) -> io::Result<String> {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut value = serde_json::to_value(record)?;
        value
            .as_object_mut()
            .ok_or_else(invalid)?
            .insert("kind".into(), kind.into());
        let payload = serde_json::to_vec(&value)?;
        if payload.len() + 1 > MAX_RECORD
            || self.file.metadata()?.len() + payload.len() as u64 + 1 > MAX_BYTES as u64
        {
            return Err(invalid());
        }
        self.file.write_all(&payload)?;
        self.file.write_all(b"\n")?;
        self.file.sync_all()?;
        File::open(&self.parent)?.sync_all()?;
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        Ok(sha256(&payload))
    }
}

pub(crate) fn consume(
    context: &CliContext,
    claim: Claim,
    deadline: Instant,
    claim_key: impl FnOnce() -> Result<(), CliError>,
) -> Result<Token, CliError> {
    if !claim.valid() {
        return Err(runtime::unavailable());
    }
    let mut ledger = Ledger::open(&context.state_dir, true, deadline)
        .map_err(error)?
        .ok_or_else(runtime::unavailable)?;
    let scan = ledger.scan(deadline).map_err(error)?;
    // Every claim permanently reserves two full slots, including unfinished
    // operations in other sessions; current file length is not free capacity.
    if scan.claims.len() >= MAX_CLAIMS || scan.claims.contains_key(&claim.op) {
        return Err(runtime::unavailable());
    }
    claim_key()?;
    let hash = ledger.append("claim", &claim, deadline).map_err(error)?;
    Ok(Token { claim, hash })
}

pub(crate) fn finish(
    context: &CliContext,
    token: &Token,
    proof: Option<&Proof>,
    reason: Reason,
    elapsed: Duration,
    deadline: Instant,
) -> Result<(), CliError> {
    let mut ledger = Ledger::open(&context.state_dir, true, deadline)
        .map_err(error)?
        .ok_or_else(runtime::unavailable)?;
    let scan = ledger.scan(deadline).map_err(error)?;
    let (_, hash) = scan
        .claims
        .get(&token.claim.op)
        .ok_or_else(runtime::unavailable)?;
    if *hash != token.hash || scan.terminals.contains_key(&token.claim.op) {
        return Err(runtime::unavailable());
    }
    let terminal = Terminal {
        schema_version: SCHEMA.into(),
        op: token.claim.op.clone(),
        claim_sha256: token.hash.clone(),
        state: if proof.is_some() {
            State::Submitted
        } else if matches!(reason, Reason::Drop) {
            State::Dropped
        } else {
            State::Unknown
        },
        native_uuid: proof.map(|p| p.row.native_uuid.clone()),
        row_offset: proof.map(|p| p.row_offset),
        row_payload_sha256: proof.map(|p| p.row_payload_sha256.clone()),
        envelope_sha256: proof.map(|_| token.claim.envelope_sha256.clone()),
        timestamp: proof.and_then(|p| p.row.timestamp.clone()),
        confirmation_elapsed_ms: proof.map(|_| elapsed.as_millis() as u64),
        reason: proof.is_none().then_some(reason),
    };
    if !terminal.valid(&token.claim) {
        return Err(runtime::unavailable());
    }
    ledger
        .append("terminal", &terminal, deadline)
        .map_err(error)?;
    Ok(())
}

pub(crate) fn confirmed(state: &Path, deadline: Instant) -> io::Result<Vec<Confirmed>> {
    let Some(mut ledger) = Ledger::open(state, false, deadline)? else {
        return Ok(Vec::new());
    };
    let scan = ledger.scan(deadline)?;
    Ok(scan
        .terminals
        .values()
        .filter_map(|terminal| terminal.confirmed(&scan.claims.get(&terminal.op)?.0))
        .collect())
}
