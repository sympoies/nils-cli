//! The session-record lock serializes publication and consumption of idle readiness.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{CliContext, CliError, SessionRecordLock, session_dir, write_private_file};

const FILE: &str = "idle-ready-boundary.json";
const MAX_RECORD_BYTES: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IdleObservation {
    /// Digest of the verified runtime, control, conversation and applied account.
    pub(crate) binding: String,
    /// Provider conversation whose observations share a watermark ordering.
    pub(crate) conversation: String,
    /// Provider-specific position that advances across completed turns.
    pub(crate) position: u64,
    /// Private digest of the provider's turn watermark.
    pub(crate) turn: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Boundary {
    pub(crate) generation: String,
    pub(crate) token: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Available,
    Invalidated,
    Consumed,
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    incarnation: String,
    generation: u64,
    token: String,
    observation: IdleObservation,
    state: State,
}

impl Record {
    fn boundary(&self) -> Boundary {
        Boundary {
            generation: self.generation.to_string(),
            token: self.token.clone(),
        }
    }
}

pub(crate) struct Store<'a> {
    path: PathBuf,
    incarnation: &'a str,
    _lock: &'a SessionRecordLock,
}

impl<'a> Store<'a> {
    /// The caller must hold the record lock for this canonical session ID.
    pub(crate) fn new(
        context: &CliContext,
        session_id: &str,
        incarnation: &'a str,
        lock: &'a SessionRecordLock,
    ) -> Self {
        Self {
            path: record_path(context, session_id),
            incarnation,
            _lock: lock,
        }
    }

    /// Accept only a fresh, positive observation under the same record lock.
    pub(crate) fn observe(
        &self,
        observation: IdleObservation,
    ) -> Result<Option<Boundary>, CliError> {
        let current = self.read()?;
        if let Some(current) = &current
            && current.observation.conversation == observation.conversation
        {
            let prior = &current.observation;
            let unchanged_turn =
                observation.position == prior.position && observation.turn == prior.turn;
            let advanced_turn =
                observation.position > prior.position && observation.turn != prior.turn;
            if !advanced_turn && (!unchanged_turn || current.state == State::Consumed) {
                self.invalidate_record(current)?;
                return Ok(None);
            }
            if current.state == State::Available && prior == &observation {
                return Ok(Some(current.boundary()));
            }
        }
        let generation = current
            .map_or(0, |record| record.generation)
            .checked_add(1)
            .ok_or_else(unavailable)?;
        let record = Record {
            incarnation: self.incarnation.into(),
            generation,
            token: uuid::Uuid::new_v4().to_string(),
            observation,
            state: State::Available,
        };
        self.write(&record)?;
        Ok(Some(record.boundary()))
    }

    pub(crate) fn invalidate(&self) -> Result<(), CliError> {
        if let Some(record) = self.read()? {
            self.invalidate_record(&record)?;
        }
        Ok(())
    }

    pub(crate) fn check(&self, token: &str) -> Result<(), CliError> {
        self.available(token).map(|_| ())
    }

    /// No input may be written until this update has been synced.
    pub(crate) fn consume(
        &self,
        token: &str,
        observation: &IdleObservation,
    ) -> Result<(), CliError> {
        let mut record = self.available(token)?;
        if &record.observation != observation {
            self.invalidate_record(&record)?;
            return Err(stale());
        }
        record.state = State::Consumed;
        self.write(&record)
    }

    fn available(&self, token: &str) -> Result<Record, CliError> {
        self.read()?
            .filter(|record| record.state == State::Available && record.token == token)
            .ok_or_else(stale)
    }

    fn read(&self) -> Result<Option<Record>, CliError> {
        Ok(read(&self.path)?.filter(|record| record.incarnation == self.incarnation))
    }

    fn invalidate_record(&self, record: &Record) -> Result<(), CliError> {
        if record.state == State::Available {
            self.write(&Record {
                state: State::Invalidated,
                ..record.clone()
            })?;
        }
        Ok(())
    }

    fn write(&self, record: &Record) -> Result<(), CliError> {
        let bytes = serde_json::to_vec(record).map_err(|_| unavailable())?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(unavailable());
        }
        write_private_file(&self.path, &bytes).map_err(|_| unavailable())?;
        File::open(self.path.parent().expect("session directory"))
            .and_then(|directory| directory.sync_all())
            .map_err(|_| unavailable())
    }
}

/// The list reads only the atomically published record. Input revalidates live
/// admission under the record lock using the supplied token.
pub(crate) fn projected(
    context: &CliContext,
    session_id: &str,
    incarnation: &str,
    binding: &str,
) -> Result<Option<Boundary>, CliError> {
    Ok(read(&record_path(context, session_id))?
        .filter(|record| {
            record.incarnation == incarnation
                && record.observation.binding == binding
                && record.state == State::Available
        })
        .map(|record| record.boundary()))
}

fn record_path(context: &CliContext, session_id: &str) -> PathBuf {
    session_dir(context, session_id).join(FILE)
}

fn read(path: &Path) -> Result<Option<Record>, CliError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unavailable()),
    };
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(unavailable());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| unavailable())
}

fn stale() -> CliError {
    CliError::data(
        "stale-idle-ready-boundary",
        "the expected idle-ready boundary is no longer available",
        Some(json!({"stage":"admission","pasted":false,"submitted":false,"dispatched":false})),
    )
}

fn unavailable() -> CliError {
    CliError::runtime(
        "idle-ready-boundary-unavailable",
        "idle-ready boundary storage is unavailable",
        None,
    )
}
