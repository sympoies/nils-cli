//! Token-free, completion-last evidence. Consumers must publish and read back
//! afresh under the accounts lock; a saved receipt is never a capability claim.
use super::remote::generation;
use super::store::{self, AuthError, AuthResult};
use nils_common::fs;
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::Path;

pub(crate) const FILE: &str = ".claude-cli-writer-receipt.json";

pub(crate) struct Context {
    generation: Value,
    attempt: String,
}

impl Context {
    pub(crate) fn from_payload(payload: &Value) -> AuthResult<Option<Self>> {
        let generation = generation::context(payload).map_err(|()| invalid())?;
        let Some(attempt) = payload.get("publication_attempt_id") else {
            return Ok(None);
        };
        let generation = generation.ok_or_else(invalid)?;
        Ok(Some(Self {
            generation: serde_json::to_value(generation).map_err(|_| invalid())?,
            attempt: attempt.as_str().ok_or_else(invalid)?.to_string(),
        }))
    }
}

fn invalid() -> AuthError {
    AuthError::data(
        "remote-export-invalid-generation",
        "publication context or attempt is invalid",
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Pending,
    Verified,
    Failed,
}

#[derive(Debug, Serialize)]
struct Outcome {
    state: State,
}

#[derive(Debug, Serialize)]
struct Stores {
    account_credentials: Outcome,
    account_config: Outcome,
}

#[derive(Clone, Copy)]
pub(crate) enum Store {
    Credentials,
    Config,
}

#[derive(Debug, Serialize)]
pub(crate) struct Receipt {
    schema_version: &'static str,
    credential_generation: Value,
    publication_attempt_id: String,
    state: &'static str,
    stores: Stores,
}

fn failure() -> AuthError {
    AuthError::runtime(
        "writer-receipt-unavailable",
        "cannot durably record writer verification",
    )
}

fn sync_dir(dir: &Path) -> AuthResult<()> {
    std::fs::File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|_| failure())
}

impl Receipt {
    pub(crate) fn begin(dir: &Path, context: &Context) -> AuthResult<Self> {
        let receipt = Self {
            schema_version: "nils.claude-writer-receipt.v1",
            credential_generation: context.generation.clone(),
            publication_attempt_id: context.attempt.clone(),
            state: "in_progress",
            stores: Stores {
                account_credentials: Outcome {
                    state: State::Pending,
                },
                account_config: Outcome {
                    state: State::Pending,
                },
            },
        };
        receipt.persist(dir)?;
        Ok(receipt)
    }

    fn persist(&self, dir: &Path) -> AuthResult<()> {
        let bytes = serde_json::to_vec(self).map_err(|_| failure())?;
        fs::write_atomic(&dir.join(FILE), &bytes, fs::SECRET_FILE_MODE).map_err(|_| failure())?;
        sync_dir(dir)
    }

    fn outcome(&mut self, store: Store) -> &mut Outcome {
        match store {
            Store::Credentials => &mut self.stores.account_credentials,
            Store::Config => &mut self.stores.account_config,
        }
    }

    pub(crate) fn complete(&mut self, dir: &Path) -> AuthResult<()> {
        if !matches!(self.stores.account_credentials.state, State::Verified)
            || !matches!(self.stores.account_config.state, State::Verified)
        {
            return Err(failure());
        }
        self.state = "completed";
        if let Err(err) = self.persist(dir) {
            // A failed fsync is ambiguous; remove any completed record before
            // returning failure. A consumer still requires a fresh publication.
            self.state = "in_progress";
            let _ = invalidate(dir);
            return Err(err);
        }
        Ok(())
    }
}

pub(crate) fn store_step(
    dir: &Path,
    receipt: &mut Option<Receipt>,
    store: Store,
    operation: impl FnOnce() -> AuthResult<()>,
) -> AuthResult<()> {
    let result = operation();
    if let Some(receipt) = receipt {
        receipt.outcome(store).state = if result.is_ok() {
            State::Verified
        } else {
            State::Failed
        };
        receipt.persist(dir)?;
    }
    result
}

pub(crate) fn verify(dir: &Path, store: Store, expected: &Map<String, Value>) -> AuthResult<()> {
    let (file, key) = match store {
        Store::Credentials => (".credentials.json", "claudeAiOauth"),
        Store::Config => (".claude.json", "oauthAccount"),
    };
    let failed = || {
        AuthError::runtime(
            "writer-readback-failed",
            "credential store readback did not match the admitted snapshot",
        )
    };
    let path = dir.join(file);
    let bytes = std::fs::read(&path).map_err(|_| failed())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| failed())?;
    if value.get(key).and_then(Value::as_object) != Some(expected) {
        return Err(failed());
    }
    // Config writes may be no-ops; explicitly persist both stores and directory
    // before a verified status becomes durable.
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| failed())?;
    sync_dir(dir)
}

/// Invalidation must itself be durable before any competing mutation proceeds.
pub(crate) fn invalidate(dir: &Path) -> AuthResult<()> {
    match std::fs::remove_file(dir.join(FILE)) {
        Ok(()) => sync_dir(dir),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(failure()),
    }
}

/// An active projection may target a managed account config directory. Join
/// its existing accounts lock before invalidating or writing those stores.
fn lock_active(
    held_accounts: Option<(&Path, &store::StoreLock)>,
) -> AuthResult<Option<store::StoreLock>> {
    let Some(dir) = store::config_dir() else {
        return Ok(None);
    };
    if dir.join(store::ACCOUNT_MARKER).is_file() {
        if let Some((held, _lock)) = held_accounts {
            let same = dir
                .parent()
                .and_then(|parent| std::fs::canonicalize(parent).ok())
                .zip(std::fs::canonicalize(held).ok())
                .is_some_and(|(actual, held)| actual == held);
            if same {
                return Ok(None);
            }
        }
        return dir.parent().map(store::lock_accounts).transpose();
    }
    Ok(None)
}

/// Fence all active-login writers, including upstream login/logout delegation.
pub(crate) fn fence_active(
    held_accounts: Option<(&Path, &store::StoreLock)>,
) -> AuthResult<Option<store::StoreLock>> {
    let lock = lock_active(held_accounts)?;
    for path in [store::credentials_file(), store::global_config_file()]
        .into_iter()
        .flatten()
    {
        if let Some(dir) = path.parent() {
            invalidate(dir)?;
        }
    }
    Ok(lock)
}

/// New receipt projections expose typed failure codes, never private paths or
/// incidental filesystem diagnostics from the existing storage helpers.
pub(crate) fn redact_error(mut error: AuthError) -> AuthError {
    error.message = "account store publication could not be verified".to_string();
    error.details = None;
    error
}
