//! Durable claims are scoped to the stable session, independent of its runtime.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{CliContext, CliError, SessionRecord, session_dir};

const SCHEMA: &str = "agent-session.send-claim.v2";

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Claim {
    schema_version: String,
    key_sha256: String,
    identity_sha256: String,
    pub(crate) evidence: Evidence,
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Evidence {
    Unknown,
    NotSubmitted {
        composer: Composer,
    },
    NativePending {
        identity_sha256: String,
        client_id: String,
        input_sha256: String,
    },
    Submitted,
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Composer {
    Absent,
    Unknown,
    Pasted {
        input_sha256: String,
        // A complete terminal observation is required for recovery.
        fence: Option<String>,
        record_updated_at: String,
    },
}

fn key_digest(key: &str) -> Result<String, CliError> {
    if key.is_empty() || key.len() > 256 {
        return Err(CliError::usage(
            "send-invalid-idempotency-key",
            "idempotency key must contain 1 to 256 bytes",
            None,
        ));
    }
    let digest = Sha256::new()
        .chain_update(b"agent-session.send-key.v1\0")
        .chain_update(key)
        .finalize();
    Ok(hex(&digest))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn input_digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn identity(record: &SessionRecord) -> Option<String> {
    let runtime = record
        .runtime
        .as_ref()
        .filter(|runtime| !runtime.launch_id.is_empty())?;
    let bytes = serde_json::to_vec(&(
        &record.id,
        &record.created_at,
        &record.agent,
        &record.tmux_session,
        &runtime.launch_id,
        runtime.generation,
        &runtime.kind,
        record
            .provider_resume
            .as_ref()
            .map(|p| (&p.provider, &p.session_id)),
    ))
    .ok()?;
    Some(hex(&Sha256::digest(bytes)))
}

fn path(context: &CliContext, record: &SessionRecord, hash: &str) -> PathBuf {
    session_dir(context, &record.id).join(format!("send-key-{hash}.json"))
}

pub(crate) fn load(
    context: &CliContext,
    record: &SessionRecord,
    key: &str,
) -> Result<Option<Claim>, CliError> {
    Ok(load_digest(context, record, &key_digest(key)?))
}

/// Find the last updated retained claim without a separate index. An exact
/// timestamp tie does not give the patrol enough evidence to pick a claim.
pub(crate) fn latest(context: &CliContext, record: &SessionRecord) -> Option<Claim> {
    let mut latest = None;
    for entry in std::fs::read_dir(session_dir(context, &record.id)).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let Some(hash) = name
            .to_str()
            .and_then(|name| name.strip_prefix("send-key-"))
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        else {
            continue;
        };
        let metadata = entry.metadata().ok()?;
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().ok()?;
        match &mut latest {
            Some((time, hash)) if *time == modified => *hash = None,
            Some((time, _)) if *time > modified => {}
            _ => latest = Some((modified, Some(hash.to_owned()))),
        }
    }
    let (_, hash) = latest?;
    load_digest(context, record, &hash?)
}

fn load_digest(context: &CliContext, record: &SessionRecord, hash: &str) -> Option<Claim> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path(context, record, hash))
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let claim: Claim = serde_json::from_slice(&bytes).ok()?;
    (claim.schema_version == SCHEMA
        && claim.key_sha256 == hash
        && claim.identity_sha256 == identity(record)?)
    .then_some(claim)
}

impl Claim {
    pub(crate) fn consume_recovery(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
    ) -> Result<(), CliError> {
        self.evidence = Evidence::Unknown;
        persist(&path(context, record, &self.key_sha256), self)
    }
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
    match path(context, record, &key_digest(key)?).try_exists() {
        Ok(true) => Err(used(record)),
        Ok(false) => Ok(()),
        Err(err) => Err(persistence_error(err)),
    }
}

/// An unfinished claim is unknown. Only a normally closed path that never
/// crossed its submit boundary can publish a negative result.
#[derive(Default)]
pub(crate) struct Attempt(Option<Writer>);

struct Writer {
    path: PathBuf,
    claim: Claim,
    before_submit: Option<Composer>,
}

pub(super) fn claim(
    context: &CliContext,
    record: &SessionRecord,
    key: Option<&str>,
) -> Result<Attempt, CliError> {
    let Some(key) = key else {
        return Ok(Attempt::default());
    };
    let key_sha256 = key_digest(key)?;
    let path = path(context, record, &key_sha256);
    let claim = Claim {
        schema_version: SCHEMA.into(),
        key_sha256,
        identity_sha256: identity(record).unwrap_or_default(),
        evidence: Evidence::Unknown,
    };
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|err| {
            if err.kind() == io::ErrorKind::AlreadyExists {
                used(record)
            } else {
                persistence_error(err)
            }
        })?;
    // A partial or failed claim remains consumed.
    file.write_all(&serialize(&claim)?)
        .map_err(persistence_error)?;
    file.sync_all().map_err(persistence_error)?;
    sync_parent(&path)?;
    Ok(Attempt(Some(Writer {
        path,
        claim,
        before_submit: Some(Composer::Absent),
    })))
}

impl Attempt {
    pub(crate) fn is_keyed(&self) -> bool {
        self.0.is_some()
    }

    pub(crate) fn composer(&mut self, composer: Composer) {
        if let Some(writer) = self.0.as_mut() {
            writer.before_submit = Some(composer);
        }
    }

    pub(crate) fn record_updated_at(&mut self, record: &SessionRecord) {
        if let Some(Composer::Pasted {
            record_updated_at, ..
        }) = self
            .0
            .as_mut()
            .and_then(|writer| writer.before_submit.as_mut())
        {
            record_updated_at.clone_from(&record.updated_at);
        }
    }

    pub(crate) fn dispatch(&mut self) {
        if let Some(writer) = self.0.as_mut() {
            writer.before_submit = None;
        }
    }

    pub(crate) fn prepare_native(
        &mut self,
        record: &SessionRecord,
        client_id: Option<&str>,
        text: &str,
    ) -> Result<(), CliError> {
        if let Some(writer) = self.0.as_mut()
            && let Some((client_id, identity_sha256)) =
                client_id.zip(crate::serve::prompt_receipts::native_identity(record))
        {
            writer.claim.evidence = Evidence::NativePending {
                identity_sha256,
                client_id: client_id.to_owned(),
                input_sha256: input_digest(text),
            };
            persist(&writer.path, &writer.claim)?;
        }
        Ok(())
    }

    pub(crate) fn submitted(&mut self) {
        if let Some(writer) = self.0.as_mut() {
            writer.before_submit = None;
            writer.claim.evidence = Evidence::Submitted;
            let _ = persist(&writer.path, &writer.claim);
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(composer) = self.before_submit.take() {
            self.claim.evidence = Evidence::NotSubmitted { composer };
            let _ = persist(&self.path, &self.claim);
        }
    }
}

fn serialize(claim: &Claim) -> Result<Vec<u8>, CliError> {
    serde_json::to_vec(claim).map_err(|err| persistence_error(err.into()))
}

fn persist(path: &Path, claim: &Claim) -> Result<(), CliError> {
    crate::write_private_file(path, &serialize(claim)?)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), CliError> {
    File::open(path.parent().expect("claim has a parent"))
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
