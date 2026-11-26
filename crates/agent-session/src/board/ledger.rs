//! Closed-session ledger (`session-board-v1`, "Closed-session ledger").
//!
//! A bounded, private record of session records that were removed, so an
//! aggregator can catch up on closes it missed. It is written by every process
//! that deletes or archives a session, whether or not the board is enabled, and
//! is exposed only through the enabled daemon routes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CliContext, CliError};

mod review_dependencies;
use review_dependencies::ReviewDependencies;

pub(crate) const LEDGER_SCHEMA: &str = "agent-session.board-closed-ledger.v1";
pub(crate) const CLOSED_SCHEMA: &str = "agent-session.board-closed.v1";
const REVIEW_RETENTION_EXTENSION: &str = "review_handoff_retention.v1";
const BOARD_DIR: &str = "board";
const LEDGER_FILE: &str = "closed-ledger.json";
const LEDGER_LOCK: &str = "closed-ledger.lock";
/// Where an unreadable, corrupt, or unsupported ledger is moved before it is
/// replaced; only the most recent one is kept.
const LEDGER_MOVED_ASIDE: &str = "closed-ledger.replaced.json";
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);
/// Ordinary feed retention: the newest entries, each at most `MAX_AGE_SECONDS` old.
pub(crate) const MAX_ENTRIES: usize = 256;
pub(crate) const MAX_AGE_SECONDS: i64 = 7 * 24 * 60 * 60;
/// Bound the ordinary and dependency-retained records in the private ledger.
const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;
const CURSOR_PREFIX: &str = "v1";

/// Why a session record was removed. `exited` and `vanished` are never written
/// by a daemon or CLI (see the spec's close reasons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseReason {
    Deleted,
    Archived,
}

impl CloseReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Deleted => "deleted",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema_version: String,
    ledger_id: String,
    last_seq: u64,
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    seq: u64,
    record: Value,
}

impl Ledger {
    fn empty() -> Self {
        Self {
            schema_version: LEDGER_SCHEMA.to_string(),
            ledger_id: uuid::Uuid::new_v4().to_string(),
            last_seq: 0,
            entries: Vec::new(),
        }
    }

    fn head(&self) -> String {
        cursor(&self.ledger_id, self.last_seq)
    }

    fn ordinary_floor(&self, now_epoch: i64) -> u64 {
        self.entries
            .iter()
            .rev()
            .filter(|entry| within_age(&entry.record, now_epoch))
            .nth(MAX_ENTRIES - 1)
            .map_or(0, |entry| entry.seq)
    }

    fn prune(&mut self, now_epoch: i64, dependencies: &ReviewDependencies) -> bool {
        let before = self.entries.len();
        let floor = self.ordinary_floor(now_epoch);
        self.entries.retain(|entry| {
            ordinary(entry, now_epoch, floor)
                || (closed_at_epoch(&entry.record).is_some()
                    && dependencies.references(&entry.record))
        });
        self.entries.len() != before
    }
}

fn ordinary(entry: &Entry, now_epoch: i64, floor: u64) -> bool {
    entry.seq >= floor && within_age(&entry.record, now_epoch)
}

fn within_age(record: &Value, now_epoch: i64) -> bool {
    closed_at_epoch(record)
        .is_some_and(|closed| now_epoch.saturating_sub(closed) <= MAX_AGE_SECONDS)
}

fn closed_at_epoch(record: &Value) -> Option<i64> {
    record
        .get("closed_at")?
        .as_str()?
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| timestamp.as_second())
}

fn cursor(ledger_id: &str, seq: u64) -> String {
    format!("{CURSOR_PREFIX}:{ledger_id}:{seq}")
}

/// A cursor this daemon could have issued, or `None`.
fn parse_cursor(raw: &str) -> Option<(String, u64)> {
    let mut parts = raw.split(':');
    let (Some(CURSOR_PREFIX), Some(id), Some(seq), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let id = uuid::Uuid::parse_str(id).ok()?;
    if seq.is_empty() || !seq.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((id.hyphenated().to_string(), seq.parse().ok()?))
}

fn unavailable() -> CliError {
    CliError::unavailable(
        "board-ledger-unavailable",
        "the closed-session ledger is unavailable",
        None,
    )
}

pub(crate) fn cursor_invalid() -> CliError {
    CliError::usage(
        "board-cursor-invalid",
        "the closed-session cursor is malformed",
        None,
    )
}

fn cursor_expired() -> CliError {
    CliError::data(
        "board-cursor-expired",
        "the closed-session cursor has expired; resynchronize from the full ledger",
        None,
    )
}

fn now_epoch() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// `<state-dir>/board`, private to the current user and canonically below the
/// state directory. An untrusted store is never repaired automatically.
fn board_root(context: &CliContext) -> Result<PathBuf, CliError> {
    let root = context.state_dir.join(BOARD_DIR);
    match fs::symlink_metadata(&root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(unavailable());
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(&root).map_err(|_| unavailable())?;
        }
        Err(_) => return Err(unavailable()),
    }
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|_| unavailable())?;
    let canonical_state = fs::canonicalize(&context.state_dir).map_err(|_| unavailable())?;
    let canonical_root = fs::canonicalize(&root).map_err(|_| unavailable())?;
    if !canonical_root.starts_with(&canonical_state) {
        return Err(unavailable());
    }
    Ok(root)
}

/// The ledger lock: bounded, like the coordination registry's.
fn lock(root: &Path) -> Result<File, CliError> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(SECRET_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join(LEDGER_LOCK))
        .map_err(|_| unavailable())?;
    let metadata = lock.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(unavailable());
    }
    let started = Instant::now();
    loop {
        // SAFETY: flock is called with a valid, owned file descriptor.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(lock);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK) || started.elapsed() >= LOCK_TIMEOUT {
            return Err(unavailable());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

enum Loaded {
    Present(Ledger),
    Missing,
    /// Unreadable content, corrupt JSON, or an unsupported version.
    Replaceable,
}

/// Read the ledger file. A wrong owner, mode, link count, or a symlink is an
/// untrusted store and fails; bad content is replaceable.
fn read_ledger(path: &Path) -> Result<Loaded, CliError> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(_) => return Err(unavailable()),
    };
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    if file
        .by_ref()
        .take(MAX_LEDGER_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_LEDGER_BYTES
    {
        return Ok(Loaded::Replaceable);
    }
    let Ok(ledger) = serde_json::from_slice::<Ledger>(&bytes) else {
        return Ok(Loaded::Replaceable);
    };
    let consistent = ledger.schema_version == LEDGER_SCHEMA
        && uuid::Uuid::parse_str(&ledger.ledger_id).is_ok()
        && ledger
            .entries
            .windows(2)
            .all(|pair| pair[0].seq < pair[1].seq)
        && ledger
            .entries
            .iter()
            .all(|entry| entry.seq >= 1 && entry.seq <= ledger.last_seq);
    Ok(if consistent {
        Loaded::Present(ledger)
    } else {
        Loaded::Replaceable
    })
}

fn write_ledger(path: &Path, ledger: &Ledger) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(ledger).map_err(|_| unavailable())?;
    if bytes.len() as u64 > MAX_LEDGER_BYTES {
        return Err(unavailable());
    }
    write_atomic(path, &bytes, SECRET_FILE_MODE).map_err(|_| unavailable())
}

/// Run `operate` on the pruned ledger under its lock, creating or replacing
/// the file as needed and writing it back when anything changed.
fn with_ledger<T>(
    context: &CliContext,
    operate: impl FnOnce(&mut Ledger, i64, &ReviewDependencies) -> Result<(T, bool), CliError>,
) -> Result<T, CliError> {
    let root = board_root(context)?;
    let _lock = lock(&root)?;
    let dependencies = ReviewDependencies::load(&root)?;
    let now = now_epoch();
    let path = root.join(LEDGER_FILE);
    let (mut ledger, mut changed) = match read_ledger(&path)? {
        Loaded::Present(ledger) => (ledger, false),
        Loaded::Missing => (Ledger::empty(), true),
        Loaded::Replaceable => {
            fs::rename(&path, root.join(LEDGER_MOVED_ASIDE)).map_err(|_| unavailable())?;
            (Ledger::empty(), true)
        }
    };
    changed |= ledger.prune(now, &dependencies);
    let (value, operated) = operate(&mut ledger, now, &dependencies)?;
    if changed || operated {
        write_ledger(&path, &ledger)?;
    }
    Ok(value)
}

/// Append one closed record. The caller has already committed the removal;
/// an error here must never fail, delay, or roll it back.
pub(crate) fn append(context: &CliContext, mut record: Value) -> Result<(), CliError> {
    with_ledger(context, |ledger, now, dependencies| {
        if let Some(object) = record.as_object_mut() {
            object.insert(
                "closed_at".to_string(),
                Value::String(jiff::Timestamp::now().to_string()),
            );
        }
        ledger.last_seq = ledger.last_seq.checked_add(1).ok_or_else(unavailable)?;
        ledger.entries.push(Entry {
            seq: ledger.last_seq,
            record,
        });
        ledger.prune(now, dependencies);
        Ok(((), true))
    })
}

/// The ledger head as an opaque cursor.
pub(crate) fn head_cursor(context: &CliContext) -> Result<String, CliError> {
    with_ledger(context, |ledger, _, _| Ok((ledger.head(), false)))
}

fn project_entry(ledger: &Ledger, entry: &Entry, machine: &str) -> Result<Value, CliError> {
    let mut record = entry.record.clone();
    let object = record.as_object_mut().ok_or_else(unavailable)?;
    if !object.contains_key("launch_env") {
        return Err(unavailable());
    }
    object.insert("machine".to_string(), Value::String(machine.to_string()));
    for key in ["role", "lineage", "work", "model", "reasoning_effort"] {
        object.entry(key).or_insert(Value::Null);
    }
    Ok(json!({ "cursor": cursor(&ledger.ledger_id, entry.seq), "record": record }))
}

pub(super) fn review_evidence_extensions() -> Vec<&'static str> {
    super::EXTENSIONS
        .into_iter()
        .chain([REVIEW_RETENTION_EXTENSION])
        .collect()
}

/// The ordinary tail and independent review retention snapshot, each stamped
/// with `machine`, as `agent-session.board-closed.v1` data.
pub(crate) fn read_since(
    context: &CliContext,
    since: Option<&str>,
    machine: &str,
) -> Result<Value, CliError> {
    let after = since
        .map(|raw| parse_cursor(raw).ok_or_else(cursor_invalid))
        .transpose()?;
    with_ledger(context, |ledger, now, dependencies| {
        let floor = ledger.ordinary_floor(now);
        let ordinary_entries = || {
            ledger
                .entries
                .iter()
                .filter(|entry| ordinary(entry, now, floor))
        };
        let after_seq = match after {
            None => 0,
            Some((ledger_id, seq)) => {
                if ledger_id != ledger.ledger_id || seq > ledger.last_seq {
                    return Err(cursor_expired());
                }
                // Every seq after the cursor must still be retained.
                let retained_after =
                    ordinary_entries().filter(|entry| entry.seq > seq).count() as u64;
                if retained_after != ledger.last_seq - seq {
                    return Err(cursor_expired());
                }
                seq
            }
        };
        let entries: Vec<Value> = ordinary_entries()
            .filter(|entry| entry.seq > after_seq)
            .map(|entry| project_entry(ledger, entry, machine))
            .collect::<Result<_, CliError>>()?;
        let retained_for_review: Vec<Value> = ledger
            .entries
            .iter()
            .filter(|entry| dependencies.references(&entry.record))
            .map(|entry| project_entry(ledger, entry, machine))
            .collect::<Result<_, CliError>>()?;
        let next_cursor = entries
            .last()
            .and_then(|entry| entry["cursor"].as_str().map(str::to_string))
            .unwrap_or_else(|| ledger.head());
        Ok((
            json!({
                "schema_version": CLOSED_SCHEMA,
                "record_schema": super::RECORD_SCHEMA,
                "extensions": review_evidence_extensions(),
                "machine": machine,
                "generated_at": jiff::Timestamp::now().to_string(),
                "entries": entries,
                "retained_for_review": retained_for_review,
                "next_cursor": next_cursor,
            }),
            false,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn fresh() -> (tempfile::TempDir, CliContext) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        (tmp, context)
    }

    fn record(id: &str) -> Value {
        json!({"session_id": id, "state": "closed", "close_reason": "deleted", "closed_at": null, "launch_env": {}})
    }

    fn ledger_path(context: &CliContext) -> PathBuf {
        context.state_dir.join("board").join(LEDGER_FILE)
    }

    fn set_dependencies(context: &CliContext, session_ids: &[&str]) {
        let digests: Vec<_> = session_ids
            .iter()
            .map(|id| {
                format!(
                    "sha256:{}",
                    crate::coordination::digest_bytes(id.as_bytes())
                )
            })
            .collect();
        write_atomic(
            &context
                .state_dir
                .join("board/review-handoff-coordinators.json"),
            &serde_json::to_vec(&json!({
                "schema_version": "agent-session.board-review-handoff-coordinators.v1",
                "coordinator_digests": digests,
            }))
            .unwrap(),
            SECRET_FILE_MODE,
        )
        .expect("write dependency snapshot");
    }

    fn read(context: &CliContext, since: Option<&str>) -> Result<Value, CliError> {
        read_since(context, since, "host-a")
    }

    fn ids(closed: &Value) -> Vec<String> {
        closed["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| {
                entry["record"]["session_id"]
                    .as_str()
                    .expect("id")
                    .to_string()
            })
            .collect()
    }

    fn rewrite(context: &CliContext, edit: impl FnOnce(&mut Value)) {
        let path = ledger_path(context);
        let mut ledger: Value =
            serde_json::from_slice(&fs::read(&path).expect("ledger")).expect("ledger json");
        edit(&mut ledger);
        write_atomic(
            &path,
            &serde_json::to_vec(&ledger).expect("ledger bytes"),
            SECRET_FILE_MODE,
        )
        .expect("rewrite ledger");
    }

    #[test]
    fn empty_ledger_head_round_trips_as_a_cursor() {
        let (_tmp, context) = fresh();
        let head = head_cursor(&context).expect("head");
        let closed = read(&context, Some(&head)).expect("read at head");
        assert_eq!(closed["entries"], json!([]));
        assert_eq!(closed["next_cursor"], head);
        let all = read(&context, None).expect("read all");
        assert_eq!(all["next_cursor"], head);
        assert_eq!(all["schema_version"], CLOSED_SCHEMA);
        assert_eq!(all["machine"], "host-a");
        let mode = fs::metadata(ledger_path(&context)).expect("ledger").mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(context.state_dir.join("board"))
            .expect("dir")
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn entries_follow_the_cursor_in_seq_order_and_carry_the_serving_machine() {
        let (_tmp, context) = fresh();
        let head = head_cursor(&context).expect("head");
        for id in ["a", "b", "c"] {
            append(&context, record(id)).expect("append");
        }
        let all = read(&context, Some(&head)).expect("read");
        assert_eq!(ids(&all), vec!["a", "b", "c"]);
        let first = all["entries"][0].clone();
        assert_eq!(first["record"]["machine"], "host-a");
        assert!(first["record"]["closed_at"].as_str().is_some());
        let after_first = read(&context, first["cursor"].as_str()).expect("after first");
        assert_eq!(ids(&after_first), vec!["b", "c"]);
        let tail = read(&context, all["next_cursor"].as_str()).expect("tail");
        assert_eq!(ids(&tail), Vec::<String>::new());
        assert_eq!(tail["next_cursor"], all["next_cursor"]);
        // The stored record never carries a machine.
        let stored: Value =
            serde_json::from_slice(&fs::read(ledger_path(&context)).unwrap()).unwrap();
        assert_eq!(stored["entries"][0]["record"].get("machine"), None);
        assert_eq!(stored["last_seq"], 3);
    }

    #[test]
    fn rejects_closed_record_missing_launch_env_without_rewriting_history() {
        let (_tmp, context) = fresh();
        let before = head_cursor(&context).expect("head");
        append(&context, record("old")).expect("append");
        let after = head_cursor(&context).expect("head after append");
        rewrite(&context, |ledger| {
            ledger["entries"][0]["record"]
                .as_object_mut()
                .unwrap()
                .remove("launch_env");
        });
        let original = fs::read(ledger_path(&context)).unwrap();
        for since in [None, Some(before.as_str()), None] {
            let error = read(&context, since).expect_err("missing launch_env must fail");
            assert_eq!(error.into_inner().code, "board-ledger-unavailable");
            assert_eq!(fs::read(ledger_path(&context)).unwrap(), original);
            assert!(
                !context
                    .state_dir
                    .join("board")
                    .join(LEDGER_MOVED_ASIDE)
                    .exists()
            );
        }
        // A cursor past the rejected entry still has a continuous empty tail.
        let tail = read(&context, Some(&after)).expect("tail");
        assert_eq!(tail["entries"], json!([]));
        assert_eq!(tail["next_cursor"], after);
        assert_eq!(fs::read(ledger_path(&context)).unwrap(), original);
    }

    #[test]
    fn current_closed_producer_preserves_launch_env_through_repeated_reads() {
        for launch_env in [json!({}), json!({"AGENT_RUNTIME_SUPPRESS_MEMORY": "1"})] {
            for reason in [CloseReason::Deleted, CloseReason::Archived] {
                let (_tmp, context) = fresh();
                let session: crate::SessionRecord = serde_json::from_value(json!({
                    "schema_version": crate::SESSION_DOCUMENT_VERSION,
                    "id": "closing", "agent": "codex", "mode": "interactive",
                    "title": "Closing", "cwd": "/srv/project", "tmux_session": "closing",
                    "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
                    "launch_env": launch_env
                }))
                .expect("session");
                let produced =
                    super::super::closed_record(&context, &session, reason).expect("producer");
                assert_eq!(produced["launch_env"], launch_env);
                super::super::record_close(&context, Some(produced));
                let original = fs::read(ledger_path(&context)).expect("stored producer output");
                let first = read(&context, None).expect("read producer output");
                assert_eq!(first["entries"][0]["record"]["launch_env"], launch_env);
                assert_eq!(
                    first["entries"][0]["record"]["close_reason"],
                    reason.as_str()
                );
                let repeated = read(&context, None).expect("repeat");
                assert_eq!(repeated["entries"], first["entries"]);
                assert_eq!(repeated["next_cursor"], first["next_cursor"]);
                assert_eq!(fs::read(ledger_path(&context)).unwrap(), original);
            }
        }
    }

    #[test]
    fn authenticated_feed_retains_an_existing_close_while_a_handoff_depends_on_it() {
        let (_tmp, context) = fresh();
        let coordinator = "00000000-0000-4000-8000-000000000007";
        let unrelated = "00000000-0000-4000-8000-000000000008";
        let mut row = record(coordinator);
        row["last_terminal_activity_at"] = json!("2020-01-01T00:00:00Z");
        append(&context, row).expect("close coordinator");
        append(&context, record(unrelated)).expect("close unrelated session");
        let head = head_cursor(&context).expect("head");
        rewrite(&context, |ledger| {
            for entry in ledger["entries"].as_array_mut().unwrap() {
                entry["record"]["closed_at"] = json!("2020-01-02T00:00:00Z");
            }
        });
        set_dependencies(&context, &[coordinator]);

        let closed = read(&context, None).expect("full closed feed");
        assert_eq!(ids(&closed), Vec::<String>::new());
        let retained = &closed["retained_for_review"][0];
        assert_eq!(retained["record"]["session_id"], coordinator);
        assert_eq!(retained["record"]["closed_at"], "2020-01-02T00:00:00Z");
        assert_eq!(
            retained["record"]["last_terminal_activity_at"],
            "2020-01-01T00:00:00Z"
        );
        assert_eq!(retained["record"]["close_reason"], "deleted");
        assert_eq!(retained["record"]["machine"], "host-a");
        assert_eq!(closed["next_cursor"], head);
        let tail = read(&context, Some(&head)).expect("read at head");
        assert_eq!(tail["retained_for_review"], closed["retained_for_review"]);
        let stored = fs::read(ledger_path(&context)).expect("pruned ledger");
        let stored: Ledger = serde_json::from_slice(&stored).unwrap();
        assert_eq!(stored.entries.len(), 1);
        assert_eq!(stored.entries[0].seq, 1);

        set_dependencies(&context, &[]);
        let released = read(&context, None).expect("read after last dependency ends");
        assert_eq!(released["retained_for_review"], json!([]));
        assert_eq!(released["entries"], json!([]));
        set_dependencies(&context, &[coordinator]);
        let pruned = read(&context, None).expect("read after a new dependency");
        assert_eq!(pruned["retained_for_review"], json!([]));
    }

    #[test]
    fn pre_upgrade_closed_records_expose_nullable_model_settings() {
        let (_tmp, context) = fresh();
        append(&context, record("pre-upgrade")).expect("append pre-upgrade");
        let mut current = record("current");
        current["model"] = json!("example-model");
        current["reasoning_effort"] = json!("high");
        append(&context, current).expect("append current");
        let all = read(&context, None).expect("read");
        assert_eq!(all["record_schema"], super::super::RECORD_SCHEMA);
        for key in ["model", "reasoning_effort"] {
            assert_eq!(all["entries"][0]["record"].get(key), Some(&Value::Null));
        }
        assert_eq!(all["entries"][1]["record"]["model"], "example-model");
        assert_eq!(all["entries"][1]["record"]["reasoning_effort"], "high");
        let stored: Value =
            serde_json::from_slice(&fs::read(ledger_path(&context)).unwrap()).unwrap();
        assert_eq!(stored["entries"][0]["record"].get("model"), None);
        assert_eq!(stored["entries"][0]["record"].get("reasoning_effort"), None);
    }

    #[test]
    fn only_a_malformed_cursor_is_invalid() {
        let (_tmp, context) = fresh();
        for raw in [
            "",
            "garbage",
            "v1:not-a-uuid:1",
            "v2:00000000-0000-4000-8000-000000000000:1",
            "v1:00000000-0000-4000-8000-000000000000:-1",
            "v1:00000000-0000-4000-8000-000000000000:1:2",
            "v1:00000000-0000-4000-8000-000000000000:",
        ] {
            let error = read(&context, Some(raw)).expect_err(raw).into_inner();
            assert_eq!(error.code, "board-cursor-invalid", "{raw}");
        }
        // Well-formed, but from another ledger: expired, not invalid.
        let error = read(&context, Some("v1:00000000-0000-4000-8000-000000000000:0"))
            .expect_err("foreign ledger")
            .into_inner();
        assert_eq!(error.code, "board-cursor-expired");
    }

    #[test]
    fn a_cursor_ahead_of_the_head_or_from_another_ledger_is_expired() {
        let (_tmp, context) = fresh();
        append(&context, record("a")).expect("append");
        let head = head_cursor(&context).expect("head");
        let (ledger_id, seq) = parse_cursor(&head).expect("own cursor");
        let ahead = cursor(&ledger_id, seq + 1);
        let error = read(&context, Some(&ahead))
            .expect_err("ahead")
            .into_inner();
        assert_eq!(error.code, "board-cursor-expired");

        // A ledger id change is checked before the seq bound: a cursor from
        // another ledger at seq 0 is expired even though 0 is not ahead.
        let other = cursor("11111111-1111-4111-8111-111111111111", 0);
        let error = read(&context, Some(&other))
            .expect_err("other")
            .into_inner();
        assert_eq!(error.code, "board-cursor-expired");
    }

    #[test]
    fn the_newest_256_ordinary_entries_are_kept_and_older_cursors_expire() {
        let (_tmp, context) = fresh();
        let head = head_cursor(&context).expect("head");
        set_dependencies(&context, &["s0"]);
        for index in 0..=MAX_ENTRIES {
            append(&context, record(&format!("s{index}"))).expect("append");
        }
        let all = read(&context, None).expect("read");
        let ids = ids(&all);
        assert_eq!(ids.len(), MAX_ENTRIES);
        assert_eq!(ids.first().map(String::as_str), Some("s1"));
        assert_eq!(all["retained_for_review"][0]["record"]["session_id"], "s0");
        let error = read(&context, Some(&head))
            .expect_err("pruned")
            .into_inner();
        assert_eq!(error.code, "board-cursor-expired");
        // The cursor just before the oldest retained entry is still whole.
        let (ledger_id, _) = parse_cursor(&head).expect("cursor");
        let edge = read(&context, Some(&cursor(&ledger_id, 1))).expect("edge");
        assert_eq!(edge["entries"].as_array().map(Vec::len), Some(MAX_ENTRIES));
    }

    #[test]
    fn entries_older_than_seven_days_are_pruned_on_read() {
        let (_tmp, context) = fresh();
        let head = head_cursor(&context).expect("head");
        append(&context, record("old")).expect("append");
        append(&context, record("new")).expect("append");
        let old = jiff::Timestamp::from_second(now_epoch() - MAX_AGE_SECONDS - 60)
            .expect("timestamp")
            .to_string();
        rewrite(&context, |ledger| {
            ledger["entries"][0]["record"]["closed_at"] = json!(old);
            ledger["entries"][0]["record"]
                .as_object_mut()
                .unwrap()
                .remove("launch_env");
        });
        let all = read(&context, None).expect("read");
        assert_eq!(ids(&all), vec!["new"]);
        let error = read(&context, Some(&head))
            .expect_err("pruned")
            .into_inner();
        assert_eq!(error.code, "board-cursor-expired");
        let stored: Value =
            serde_json::from_slice(&fs::read(ledger_path(&context)).unwrap()).unwrap();
        assert_eq!(stored["entries"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn corrupt_or_unsupported_ledgers_are_replaced_on_read_and_append() {
        for bad in [
            b"{".to_vec(),
            serde_json::to_vec(&json!({
                "schema_version": "agent-session.board-closed-ledger.v2",
                "ledger_id": "00000000-0000-4000-8000-000000000000",
                "last_seq": 0,
                "entries": []
            }))
            .unwrap(),
        ] {
            for use_append in [false, true] {
                let (_tmp, context) = fresh();
                append(&context, record("before")).expect("append");
                let old_head = head_cursor(&context).expect("head");
                write_atomic(&ledger_path(&context), &bad, SECRET_FILE_MODE).expect("corrupt");
                if use_append {
                    append(&context, record("after")).expect("append replaces");
                }
                let all = read(&context, None).expect("read replaces");
                let expected: Vec<String> = if use_append {
                    vec!["after".to_string()]
                } else {
                    Vec::new()
                };
                assert_eq!(ids(&all), expected);
                let error = read(&context, Some(&old_head))
                    .expect_err("new ledger id")
                    .into_inner();
                assert_eq!(error.code, "board-cursor-expired");
                assert!(
                    context
                        .state_dir
                        .join("board")
                        .join(LEDGER_MOVED_ASIDE)
                        .is_file()
                );
            }
        }
    }

    #[test]
    fn an_untrusted_store_is_unavailable_and_never_repaired() {
        let (tmp, context) = fresh();
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).expect("elsewhere");
        std::os::unix::fs::symlink(&elsewhere, context.state_dir.join("board")).expect("symlink");
        for result in [
            append(&context, record("a")).map(|_| Value::Null),
            head_cursor(&context).map(Value::String),
            read(&context, None),
        ] {
            assert_eq!(
                result.expect_err("untrusted").into_inner().code,
                "board-ledger-unavailable"
            );
        }
        assert!(!elsewhere.join(LEDGER_FILE).exists());

        let (_tmp, context) = fresh();
        head_cursor(&context).expect("create");
        fs::set_permissions(ledger_path(&context), fs::Permissions::from_mode(0o644))
            .expect("loosen mode");
        assert_eq!(
            read(&context, None)
                .expect_err("loose mode")
                .into_inner()
                .code,
            "board-ledger-unavailable"
        );
    }
}
