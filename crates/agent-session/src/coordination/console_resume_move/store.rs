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
const OPERATION_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema_version: String,
    moves: BTreeMap<String, Receipt>,
    operations: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    digest: String,
    request: String,
    outcome: Outcome,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    InFlight,
    Unknown,
    Created { reply: Reply },
    Refused { reply: Reply },
}

impl Outcome {
    fn terminal(&self) -> Option<&Reply> {
        match self {
            Self::Created { reply } | Self::Refused { reply } => Some(reply),
            Self::InFlight | Self::Unknown => None,
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
    Unavailable,
}

impl Error {
    pub(super) fn reply(self, move_id: &str) -> Reply {
        let failure = match self {
            Self::Conflict => {
                Failure::before_start(StatusCode::FORBIDDEN, "resume-move-binding-mismatch")
            }
            Self::Unavailable => Failure::unresolved(
                StatusCode::SERVICE_UNAVAILABLE,
                "resume-move-state-unavailable",
            ),
        };
        Reply::failure(failure, Some(move_id))
    }
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
        let move_id = &request.session.move_id;
        if self
            .index
            .operations
            .get(&request.operation_id)
            .is_some_and(|id| id != move_id)
        {
            return Err(Error::Conflict);
        }
        let Some(receipt) = self.index.moves.get(move_id) else {
            if self.index.operations.contains_key(&request.operation_id) {
                return Err(Error::Unavailable);
            }
            return Ok(None);
        };
        let original =
            StartRequest::parse(receipt.request.as_bytes()).map_err(|_| Error::Unavailable)?;
        if original.digest() != receipt.digest || original.session.move_id != *move_id {
            return Err(Error::Unavailable);
        }
        if receipt.digest != request.digest() {
            return Err(Error::Conflict);
        }
        Ok(Some(match receipt.outcome.terminal() {
            Some(reply) => {
                let reply = reply
                    .checked_retained(&original)
                    .ok_or(Error::Unavailable)?;
                if self.bind_operation(request) {
                    self.save()?;
                }
                Retained::Terminal(reply)
            }
            None => Retained::Unresolved(receipt.request.clone()),
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
            self.index.moves.insert(
                request.session.move_id.clone(),
                Receipt {
                    digest: request.digest(),
                    request: body.into(),
                    outcome: Outcome::InFlight,
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
            .get_mut(&request.session.move_id)
            .ok_or(Error::Unavailable)?;
        if receipt.digest != request.digest() {
            return Err(Error::Unavailable);
        }
        if let Some(retained) = receipt.outcome.terminal() {
            return retained.checked_retained(request).ok_or(Error::Unavailable);
        }
        receipt.outcome = if !reply.terminal() {
            Outcome::Unknown
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

    fn bind_operation(&mut self, request: &StartRequest) -> bool {
        let operations = &mut self.index.operations;
        if operations.contains_key(&request.operation_id) {
            return false;
        }
        let cutoff_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .saturating_sub(OPERATION_AGE)
            .as_millis();
        operations.retain(|id, _| {
            operation_timestamp_ms(id).is_some_and(|timestamp| u128::from(timestamp) >= cutoff_ms)
        });
        while operations.len() >= MAX_OPERATIONS {
            operations.pop_first();
        }
        operations.insert(
            request.operation_id.clone(),
            request.session.move_id.clone(),
        );
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
