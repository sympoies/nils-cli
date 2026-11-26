//! The origin's persistent dispatch fence. The lock never spans an HTTP call.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::CliContext;

use super::{
    super::{lock_private_store, read_private_file},
    refusal::Failure,
    reply::Reply,
    wire::{StartRequest, operation_timestamp_ms},
};

const STORE: &str = "resume-move-origin.v1.json";
const LOCK: &str = "resume-move-origin.lock";
const SCHEMA: &str = "agent-session.console-resume-move-origin.v1";
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_OPERATIONS: usize = 1024;
// Bound what one authenticated caller can keep in the shared index. A live
// receipt retains its request body for reconciliation, so it is limited more
// tightly than a compacted terminal fence. A repeat of an existing move is
// never limited.
const MAX_LIVE_PER_CALLER: usize = 8;
const MAX_RECEIPTS_PER_CALLER: usize = 128;
const OPERATION_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema_version: String,
    // Both maps are scoped to the authenticated caller session ID, so one
    // caller can never claim, burn or observe another caller's move or
    // operation ID. Keys are `<caller>/<move_id>` and `<caller>/<operation_id>`;
    // a session ID never contains `/`.
    moves: BTreeMap<String, Receipt>,
    operations: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    digest: String,
    outcome: Outcome,
}

// Only an unresolved receipt keeps the original request for reconciliation.
// A terminal receipt keeps just the digest fence and the validated reply.
#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    InFlight { request: String },
    Unknown { request: String },
    Created { reply: Reply },
    Refused { reply: Reply },
}

impl Outcome {
    fn terminal(&self) -> Option<&Reply> {
        match self {
            Self::Created { reply } | Self::Refused { reply } => Some(reply),
            Self::InFlight { .. } | Self::Unknown { .. } => None,
        }
    }

    fn unresolved(&self) -> Option<&str> {
        match self {
            Self::InFlight { request } | Self::Unknown { request } => Some(request),
            Self::Created { .. } | Self::Refused { .. } => None,
        }
    }
}

pub(super) enum Retained {
    Unresolved(String),
    Terminal(Reply),
}

#[derive(Clone, Copy)]
pub(super) enum Error {
    Conflict,
    Quota,
    Unavailable,
}

impl Error {
    pub(super) fn reply(self, move_id: &str) -> Reply {
        let failure = match self {
            // A conflict says nothing about the original start's outcome, so it
            // is an unresolved result with no `session_created` claim.
            Self::Conflict => {
                Failure::unresolved(StatusCode::FORBIDDEN, "resume-move-binding-mismatch")
            }
            // Refused before any claim or forward, so no move was created.
            Self::Quota => Failure::before_start(
                StatusCode::SERVICE_UNAVAILABLE,
                "resume-move-state-unavailable",
            ),
            Self::Unavailable => Failure::unresolved(
                StatusCode::SERVICE_UNAVAILABLE,
                "resume-move-state-unavailable",
            ),
        };
        Reply::failure(failure, Some(move_id))
    }
}

fn keys(request: &StartRequest) -> (String, String) {
    let caller = request.caller_id();
    (
        format!("{caller}/{}", request.session.move_id),
        format!("{caller}/{}", request.operation_id),
    )
}

fn operation_key_timestamp_ms(key: &str) -> Option<u64> {
    operation_timestamp_ms(key.split_once('/')?.1)
}

pub(super) struct Store {
    directory: PathBuf,
    lock: File,
    index: Index,
}

impl Store {
    pub(super) fn open(context: &CliContext) -> Result<Self, Error> {
        let lock = lock_private_store(context, LOCK).map_err(|_| Error::Unavailable)?;
        let directory = context.state_dir.join("coordination");
        let index = match read_private_file(&directory.join(STORE), MAX_STORE_BYTES) {
            Ok(bytes) => serde_json::from_slice::<Index>(&bytes).map_err(|_| Error::Unavailable)?,
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && lock.metadata().map_err(|_| Error::Unavailable)?.len() == 0 =>
            {
                Index {
                    schema_version: SCHEMA.into(),
                    moves: BTreeMap::new(),
                    operations: BTreeMap::new(),
                }
            }
            Err(_) => return Err(Error::Unavailable),
        };
        if index.schema_version != SCHEMA {
            return Err(Error::Unavailable);
        }
        Ok(Self {
            directory,
            lock,
            index,
        })
    }

    // Observe a terminal or unresolved receipt before negotiating with the relay.
    pub(super) fn existing(&mut self, request: &StartRequest) -> Result<Option<Retained>, Error> {
        let (move_key, operation_key) = keys(request);
        if self
            .index
            .operations
            .get(&operation_key)
            .is_some_and(|id| *id != request.session.move_id)
        {
            return Err(Error::Conflict);
        }
        let Some(receipt) = self.index.moves.get(&move_key) else {
            if self.index.operations.contains_key(&operation_key) {
                return Err(Error::Unavailable);
            }
            return Ok(None);
        };
        if let Some(body) = receipt.outcome.unresolved() {
            let original = StartRequest::parse(body.as_bytes()).map_err(|_| Error::Unavailable)?;
            if original.digest() != receipt.digest || keys(&original).0 != move_key {
                return Err(Error::Unavailable);
            }
        }
        if receipt.digest != request.digest() {
            return Err(Error::Conflict);
        }
        Ok(Some(match receipt.outcome.terminal() {
            Some(reply) => {
                let reply = reply.checked_retained(request).ok_or(Error::Unavailable)?;
                if self.bind_operation(request) {
                    self.save()?;
                }
                Retained::Terminal(reply)
            }
            None => Retained::Unresolved(
                receipt
                    .outcome
                    .unresolved()
                    .expect("a non-terminal receipt is unresolved")
                    .to_owned(),
            ),
        }))
    }

    /// Returns the prior fence, or persists a first claim before returning None.
    pub(super) fn claim(
        mut self,
        request: &StartRequest,
        body: &str,
    ) -> Result<Option<Retained>, Error> {
        let retained = self.existing(request)?;
        if matches!(retained, Some(Retained::Terminal(_))) {
            return Ok(retained);
        }
        if retained.is_none() {
            self.admit(request.caller_id())?;
            self.index.moves.insert(
                keys(request).0,
                Receipt {
                    digest: request.digest(),
                    outcome: Outcome::InFlight {
                        request: body.into(),
                    },
                },
            );
        }
        if self.bind_operation(request) || retained.is_none() {
            self.save()?;
        }
        Ok(retained)
    }

    pub(super) fn record(mut self, request: &StartRequest, reply: Reply) -> Result<Reply, Error> {
        let receipt = self
            .index
            .moves
            .get_mut(&keys(request).0)
            .ok_or(Error::Unavailable)?;
        if receipt.digest != request.digest() {
            return Err(Error::Unavailable);
        }
        if let Some(retained) = receipt.outcome.terminal() {
            return retained.checked_retained(request).ok_or(Error::Unavailable);
        }
        let unresolved = receipt.outcome.unresolved().map(str::to_owned);
        receipt.outcome = if !reply.terminal() {
            Outcome::Unknown {
                request: unresolved.ok_or(Error::Unavailable)?,
            }
        } else if reply.is_created() {
            Outcome::Created {
                reply: reply.clone(),
            }
        } else {
            Outcome::Refused {
                reply: reply.clone(),
            }
        };
        self.save()?;
        Ok(reply)
    }

    // A first claim for one caller must fit its quota. Other callers' quotas
    // and every repeat of an existing move are unaffected.
    fn admit(&self, caller: &str) -> Result<(), Error> {
        let (mut live, mut total) = (0, 0);
        let prefix = format!("{caller}/");
        for receipt in self
            .index
            .moves
            .range(prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&prefix))
            .map(|(_, receipt)| receipt)
        {
            total += 1;
            live += usize::from(receipt.outcome.terminal().is_none());
        }
        if live >= MAX_LIVE_PER_CALLER || total >= MAX_RECEIPTS_PER_CALLER {
            return Err(Error::Quota);
        }
        Ok(())
    }

    fn bind_operation(&mut self, request: &StartRequest) -> bool {
        let key = keys(request).1;
        let operations = &mut self.index.operations;
        if operations.contains_key(&key) {
            return false;
        }
        let cutoff_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .saturating_sub(OPERATION_AGE)
            .as_millis();
        operations.retain(|key, _| {
            operation_key_timestamp_ms(key)
                .is_some_and(|timestamp| u128::from(timestamp) >= cutoff_ms)
        });
        // Evict the oldest binding across callers, not the first key.
        while operations.len() >= MAX_OPERATIONS {
            let Some(oldest) = operations
                .keys()
                .min_by_key(|key| operation_key_timestamp_ms(key))
                .cloned()
            else {
                break;
            };
            operations.remove(&oldest);
        }
        operations.insert(key, request.session.move_id.clone());
        true
    }

    fn save(&mut self) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&self.index).map_err(|_| Error::Unavailable)?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(Error::Unavailable);
        }
        write_atomic(&self.directory.join(STORE), &bytes, SECRET_FILE_MODE)
            .map_err(|_| Error::Unavailable)?;
        // Preserve independent evidence that this index existed. A missing
        // index after any prior forward must fail closed on the same lock.
        if self.lock.metadata().map_err(|_| Error::Unavailable)?.len() == 0 {
            self.lock.write_all(b"1").map_err(|_| Error::Unavailable)?;
            self.lock.sync_all().map_err(|_| Error::Unavailable)?;
        }
        File::open(&self.directory)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| Error::Unavailable)
    }
}

#[cfg(test)]
mod tests;
