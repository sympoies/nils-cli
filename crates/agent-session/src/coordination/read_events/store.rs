//! Append-only per-conversation ledger of mailbox read events.
//!
//! One JSON line per change under `<state>/history/mailbox-reads/`. A reader
//! folds lines by message id, so replay is idempotent and a torn final line
//! (no newline yet) is ignored.
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_LEDGER_BYTES: u64 = 32 * 1024 * 1024;
const LOCK_ATTEMPTS: u32 = 5;
const LOCK_RETRY: std::time::Duration = std::time::Duration::from_millis(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Read,
    Acknowledge,
}

/// What the mailbox registry says about the message at record time.
pub(crate) struct Source {
    pub(crate) message_id: String,
    pub(crate) sender: Value,
    pub(crate) category: String,
    pub(crate) body: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
enum Line {
    Create {
        message_id: String,
        sender: Value,
        category: String,
        body: String,
        anchor: u64,
        created_at_ms: i64,
        acknowledged: bool,
    },
    Update {
        message_id: String,
        read_at_ms: Option<i64>,
        acknowledged_at_ms: Option<i64>,
    },
}

/// A folded event in creation order.
#[derive(Clone, Debug)]
pub(crate) struct Event {
    pub(crate) message_id: String,
    pub(crate) sender: Value,
    pub(crate) category: String,
    pub(crate) body: String,
    pub(crate) anchor: u64,
    pub(crate) created_at_ms: i64,
    pub(crate) read_at_ms: Option<i64>,
    pub(crate) acknowledged_at_ms: Option<i64>,
    pub(crate) revision: u64,
}

/// Contention means another writer is mid-append; recording is display-only,
/// so a short bounded wait then skip, never a block on the committed read.
fn lock_bounded(file: &fs::File) -> io::Result<()> {
    for attempt in 0..LOCK_ATTEMPTS {
        // SAFETY: flock only observes the descriptor owned by `file`; the lock
        // is released when the descriptor closes.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
        if attempt + 1 < LOCK_ATTEMPTS {
            std::thread::sleep(LOCK_RETRY);
        }
    }
    Err(io::ErrorKind::WouldBlock.into())
}

fn ledger_path(state_dir: &Path, history_id: &str) -> PathBuf {
    state_dir
        .join("history/mailbox-reads")
        .join(format!("{history_id}.jsonl"))
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

pub(crate) fn record(
    state_dir: &Path,
    history_id: &str,
    source: &Source,
    anchor: u64,
    action: Action,
) -> io::Result<()> {
    let path = ledger_path(state_dir, history_id);
    let directory = path.parent().expect("ledger path has a parent");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .open(&path)?;
    lock_bounded(&file)?;
    let events = fold(&read_all(&mut file)?)?;
    let existing = events
        .iter()
        .find(|event| event.message_id == source.message_id);
    let now = now_ms();
    let line = match (existing, action) {
        (None, _) => Line::Create {
            message_id: source.message_id.clone(),
            sender: source.sender.clone(),
            category: source.category.clone(),
            body: source.body.clone(),
            anchor,
            created_at_ms: now,
            acknowledged: action == Action::Acknowledge,
        },
        (Some(event), Action::Read) if event.read_at_ms.is_none() => Line::Update {
            message_id: source.message_id.clone(),
            read_at_ms: Some(now),
            acknowledged_at_ms: None,
        },
        (Some(event), Action::Acknowledge) if event.acknowledged_at_ms.is_none() => Line::Update {
            message_id: source.message_id.clone(),
            read_at_ms: None,
            acknowledged_at_ms: Some(now),
        },
        _ => return Ok(()),
    };
    let mut bytes = serde_json::to_vec(&line).map_err(io::Error::other)?;
    bytes.push(b'\n');
    file.write_all(&bytes)
}

pub(crate) fn load(state_dir: &Path, history_id: &str) -> io::Result<Vec<Event>> {
    let mut file = match fs::File::open(ledger_path(state_dir, history_id)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    fold(&read_all(&mut file)?)
}

fn read_all(file: &mut fs::File) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(MAX_LEDGER_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LEDGER_BYTES {
        return Err(io::Error::other("mailbox read ledger too large"));
    }
    Ok(bytes)
}

fn fold(bytes: &[u8]) -> io::Result<Vec<Event>> {
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |i| i + 1);
    let mut events: Vec<Event> = Vec::new();
    for raw in bytes[..complete]
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
    {
        match serde_json::from_slice::<Line>(raw).map_err(io::Error::other)? {
            Line::Create {
                message_id,
                sender,
                category,
                body,
                anchor,
                created_at_ms,
                acknowledged,
            } => {
                if events.iter().any(|event| event.message_id == message_id) {
                    continue;
                }
                events.push(Event {
                    message_id,
                    sender,
                    category,
                    body,
                    anchor,
                    created_at_ms,
                    read_at_ms: (!acknowledged).then_some(created_at_ms),
                    acknowledged_at_ms: acknowledged.then_some(created_at_ms),
                    revision: 1,
                });
            }
            Line::Update {
                message_id,
                read_at_ms,
                acknowledged_at_ms,
            } => {
                let Some(event) = events
                    .iter_mut()
                    .find(|event| event.message_id == message_id)
                else {
                    continue;
                };
                if event.read_at_ms.is_none() && read_at_ms.is_some() {
                    event.read_at_ms = read_at_ms;
                    event.revision += 1;
                }
                if event.acknowledged_at_ms.is_none() && acknowledged_at_ms.is_some() {
                    event.acknowledged_at_ms = acknowledged_at_ms;
                    event.revision += 1;
                }
            }
        }
    }
    Ok(events)
}
