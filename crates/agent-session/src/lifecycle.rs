//! Bounded, private lifecycle evidence. Never accepts prompts, account names,
//! capability bytes, provider arguments, terminal output or arbitrary details.
use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::{CliContext, CliError, SessionRecord};
use serde_json::{Value, json};

const SEGMENT_BYTES: u64 = 256 * 1024;
const RECORD_BYTES: usize = 8192;
thread_local! {
    static SERVE: Cell<Option<&'static str>> = const { Cell::new(None) };
    static ACTIVE: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Engine calls made inside an HTTP operation are recorded by its response
/// owner, including validation before engine entry and failures after launch.
pub(crate) struct ServeGuard(Option<&'static str>);
impl ServeGuard {
    pub(crate) fn enter(operation: &'static str) -> Self {
        Self(SERVE.with(|value| value.replace(Some(operation))))
    }
}
impl Drop for ServeGuard {
    fn drop(&mut self) {
        SERVE.with(|value| value.set(self.0));
    }
}

pub(crate) fn attempt<T>(
    context: &CliContext,
    id: &str,
    operation: &str,
    run: impl FnOnce() -> Result<T, CliError>,
) -> Result<T, CliError> {
    let before = crate::load_session_record(context, id).ok();
    let id = before.as_ref().map(|r| r.id.as_str()).unwrap_or(id);
    let key = (id.to_string(), operation.to_string());
    let serve_owner = SERVE.with(Cell::get);
    if serve_owner == Some(operation)
        || (serve_owner == Some("create") && matches!(operation, "start" | "import"))
        || ACTIVE.with(|active| active.borrow().contains(&key))
    {
        return run();
    }
    struct ActiveAttempt;
    impl Drop for ActiveAttempt {
        fn drop(&mut self) {
            ACTIVE.with(|active| {
                active.borrow_mut().pop();
            });
        }
    }
    ACTIVE.with(|active| active.borrow_mut().push(key));
    let _active = ActiveAttempt;
    let result = run();
    let after = crate::load_session_record(context, id).ok();
    record(
        context,
        after.as_ref().or(before.as_ref()),
        id,
        operation,
        if serve_owner.is_some() {
            "serve"
        } else {
            "cli"
        },
        result.as_ref().map(|_| ()).map_err(|e| e),
        None,
    );
    result
}

fn failure_message(error: &CliError) -> String {
    // Only static, audited messages are copied. Dynamic errors can contain
    // provider text, arguments, paths or credentials, and get a fixed summary.
    match error.message() {
        "the prior coordination incarnation is still live"
        | "the prior coordination runtime identity cannot be proven stopped"
        | "broker recovery requires the exact persisted runtime to be running"
        | "session runtime changed before archive"
        | "the account switch stopped the session but could not resume it; the next account is marked failed and can be retried by resuming"
        | "coordination broker is unavailable" => error.message().to_string(),
        _ => "lifecycle operation failed; inspect the returned error code".to_string(),
    }
}

fn proof_step(error: &CliError) -> &'static str {
    if let Some(step) = error
        .details()
        .and_then(|v| v.get("proof_step"))
        .and_then(Value::as_str)
    {
        match step {
            "heartbeat-fresh" => return "heartbeat-fresh",
            "broker-state" => return "broker-state",
            "process-group-probe" => return "process-group-probe",
            _ => {}
        }
    }
    match error.code() {
        "session-incarnation-conflict" => "incarnation-fence",
        "coordination-runtime-unverified" => "runtime-stopped-proof",
        "coordination-unauthorized" => "capability-present",
        "coordination-broker-not-lost" | "coordination-broker-lost" => "broker-state",
        "operation-in-progress" | "broker-replacement-grace" => "operation-quiescence",
        "session-termination-failed" => "process-group-probe",
        code if code.ends_with("incarnation-conflict") => "incarnation-fence",
        _ => "operation-validation",
    }
}

pub(crate) fn record(
    context: &CliContext,
    target: Option<&SessionRecord>,
    id: &str,
    operation: &str,
    caller: &str,
    result: Result<(), &CliError>,
    exit: Option<Value>,
) {
    let id = target.map(|r| r.id.as_str()).unwrap_or(id);
    if crate::validate_id(id).is_err() {
        return;
    }
    let runtime = target.and_then(|r| r.runtime.as_ref());
    let caller_session = if caller == "cli" {
        crate::non_empty_env("AGENT_SESSION_ID").filter(|id| crate::validate_id(id).is_ok())
    } else {
        None
    };
    let outcome = match result {
        Ok(()) => json!({"ok": true}),
        Err(error) => {
            json!({"ok": false, "code": error.code(), "message": failure_message(error), "proof_step": proof_step(error)})
        }
    };
    let row = json!({
        "schema_version": "agent-session.lifecycle.v1",
        "timestamp": jiff::Timestamp::now().to_string(),
        "operation": operation,
        "caller": {"kind": caller, "session_id": caller_session, "binary_version": env!("CARGO_PKG_VERSION"), "binary_path": std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())},
        "session_id": id,
        "session_incarnation": runtime.map(|r| &r.launch_id),
        "session_generation": runtime.map(|r| r.generation),
        "result": outcome,
        "exit": exit,
    });
    if append(context, id, &row).is_err() {
        eprintln!("warning: lifecycle-journal-unavailable");
    }
}

fn unavailable() -> CliError {
    CliError::runtime(
        "lifecycle-journal-unavailable",
        "lifecycle journal is unavailable or untrusted",
        None,
    )
}
fn private_dir(path: &Path) -> Result<(), CliError> {
    match fs::create_dir(path) {
        Ok(()) => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| unavailable())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(unavailable()),
    }
    let m = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err(unavailable());
    }
    Ok(())
}
fn open(path: &Path, create: bool) -> Result<File, CliError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| unavailable())?;
    let m = file.metadata().map_err(|_| unavailable())?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.nlink() != 1
    {
        return Err(unavailable());
    }
    Ok(file)
}
struct Journal {
    _lock: File,
    dir: PathBuf,
}
impl Journal {
    fn lock(context: &CliContext, id: &str) -> Result<Self, CliError> {
        crate::validate_id(id)?;
        fs::create_dir_all(&context.state_dir).map_err(|_| unavailable())?;
        let root = context.state_dir.join("lifecycle");
        private_dir(&root)?;
        let dir = root.join(id);
        private_dir(&dir)?;
        let lock = open(&dir.join("journal.lock"), true)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(unavailable());
        }
        Ok(Self { _lock: lock, dir })
    }
}
fn append(context: &CliContext, id: &str, row: &Value) -> Result<(), CliError> {
    let journal = Journal::lock(context, id)?;
    let path = journal.dir.join("current.jsonl");
    let marker = journal.dir.join("exit-observed.json");
    let exit_identity = json!([row["session_incarnation"], row["session_generation"]]);
    if row["operation"] == "exit-observed" && marker.exists() {
        let mut previous = String::new();
        open(&marker, false)?
            .take(RECORD_BYTES as u64)
            .read_to_string(&mut previous)
            .map_err(|_| unavailable())?;
        if serde_json::from_str::<Value>(&previous).ok().as_ref() == Some(&exit_identity) {
            return Ok(());
        }
    }
    let mut bytes = serde_json::to_vec(row).map_err(|_| unavailable())?;
    bytes.push(b'\n');
    if bytes.len() > RECORD_BYTES {
        return Err(unavailable());
    }
    let mut file = open(&path, true)?;
    if file.metadata().map_err(|_| unavailable())?.len() + bytes.len() as u64 > SEGMENT_BYTES {
        // The old previous segment is replaced only while the stable lock is held.
        fs::rename(&path, journal.dir.join("previous.jsonl")).map_err(|_| unavailable())?;
        file = open(&path, true)?;
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::End(0)).map_err(|_| unavailable())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_data())
        .map_err(|_| unavailable())?;
    if row["operation"] == "exit-observed" {
        nils_common::fs::write_atomic(
            &marker,
            &serde_json::to_vec(&exit_identity).map_err(|_| unavailable())?,
            0o600,
        )
        .map_err(|_| unavailable())?;
    }
    Ok(())
}
pub(crate) fn read(context: &CliContext, id: &str, limit: usize) -> Result<Value, CliError> {
    crate::validate_id(id)?;
    if !context.state_dir.join("lifecycle").join(id).exists() {
        return Ok(json!({"session_id": id, "records": []}));
    }
    let journal = Journal::lock(context, id)?;
    let mut records = std::collections::VecDeque::new();
    for name in ["previous.jsonl", "current.jsonl"] {
        let path = journal.dir.join(name);
        if !path.exists() {
            continue;
        }
        let file = open(&path, false)?;
        if file.metadata().map_err(|_| unavailable())?.len() > SEGMENT_BYTES {
            return Err(unavailable());
        }
        let mut content = String::new();
        file.take(SEGMENT_BYTES + 1)
            .read_to_string(&mut content)
            .map_err(|_| unavailable())?;
        for line in content.lines() {
            if let Ok(row) = serde_json::from_str::<Value>(line) {
                records.push_back(row);
            }
            if records.len() > limit.min(1000) {
                records.pop_front();
            }
        }
    }
    Ok(json!({"session_id": id, "records": records}))
}

pub(crate) async fn response(
    context: &CliContext,
    id: &str,
    operation: &str,
    before: Option<SessionRecord>,
    response: axum::response::Response,
) -> axum::response::Response {
    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return crate::serve::join_err(),
    };
    let value = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    let target_id = value
        .pointer("/data/session/id")
        .and_then(Value::as_str)
        .unwrap_or(id)
        .to_owned();
    let error = value.get("error").map(|e| {
        CliError::runtime(
            e["code"].as_str().unwrap_or("serve-operation-failed"),
            e["message"].as_str().unwrap_or("serve operation failed"),
            e.get("details").cloned(),
        )
    });
    let context = context.clone();
    let operation = operation.to_owned();
    let _ = tokio::task::spawn_blocking(move || {
        let target = crate::load_session_record(&context, &target_id)
            .ok()
            .or(before);
        record(
            &context,
            target.as_ref(),
            &target_id,
            &operation,
            "serve",
            error.as_ref().map_or(Ok(()), Err),
            None,
        );
    })
    .await;

    axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
}

/// Inventory and controller probes can observe an external disappearance even
/// when the killed wrapper never got to report its exit status.
pub(crate) fn observe_stopped(context: &CliContext, target: &SessionRecord) {
    let Some(runtime) = target.runtime.as_ref() else {
        return;
    };
    let marker = context
        .state_dir
        .join("lifecycle")
        .join(&target.id)
        .join("exit-observed.json");
    // Fast path for repeated inventory snapshots: no segment scan or append.
    if let Ok(file) = open(&marker, false) {
        let mut bytes = String::new();
        if file
            .take(RECORD_BYTES as u64)
            .read_to_string(&mut bytes)
            .is_ok()
            && serde_json::from_str::<Value>(&bytes).ok()
                == Some(json!([runtime.launch_id, runtime.generation]))
        {
            return;
        }
    }
    if crate::persisted_tmux_runtime_identity(target)
        .ok()
        .flatten()
        .is_none()
    {
        return;
    }
    let stopped_by = read(context, &target.id, 1000).ok().and_then(|v| {
        v["records"].as_array().map(|rows| {
            rows.iter().any(|row| {
                row["operation"] == "stop"
                    && row["result"]["ok"] == true
                    && row["session_incarnation"].as_str()
                        == target.runtime.as_ref().map(|r| r.launch_id.as_str())
            })
        })
    });
    let known = stopped_by == Some(true);
    record(
        context,
        Some(target),
        &target.id,
        "exit-observed",
        "controller",
        Ok(()),
        Some(
            json!({"code": null, "signal": null, "reason": if known { "runtime-stopped" } else { "runtime disappeared outside agent-session" }, "stopped_by": if known { Some("agent-session") } else { None }}),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn context(dir: &Path) -> CliContext {
        CliContext {
            state_dir: dir.to_path_buf(),
            host: None,
        }
    }

    #[test]
    fn observed_disappearance_is_unknown_and_deduplicated_per_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());
        let mut target: SessionRecord = serde_json::from_value(json!({
            "schema_version": crate::SESSION_DOCUMENT_VERSION, "id":"observed", "agent":"claude",
            "mode":"interactive", "title":null, "cwd":dir.path(), "tmux_session":"fixture",
            "prompt_file":null, "log_file":null, "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z",
            "runtime":{"kind":"tmux","tmux_session":"fixture","generation":1,"started_at":"2026-01-01T00:00:00Z","launch_id":"old"},
            "delete_tmux_identity":{"launch_id":"old","session_id":"$7","pane_id":"%7","pane_pid":7,"process_group_id":7}
        })).unwrap();
        observe_stopped(&context, &target);
        observe_stopped(&context, &target);
        let rows = read(&context, "observed", 10).unwrap();
        assert_eq!(rows["records"].as_array().unwrap().len(), 1);
        assert_eq!(
            rows["records"][0]["exit"]["reason"],
            "runtime disappeared outside agent-session"
        );
        assert_eq!(rows["records"][0]["exit"]["code"], Value::Null);
        assert_eq!(rows["records"][0]["exit"]["signal"], Value::Null);
        assert_eq!(rows["records"][0]["exit"]["stopped_by"], Value::Null);
        target.runtime.as_mut().unwrap().generation = 2;
        record(
            &context,
            Some(&target),
            "observed",
            "stop",
            "cli",
            Ok(()),
            None,
        );
        observe_stopped(&context, &target);
        let rows = read(&context, "observed", 10).unwrap();
        assert_eq!(rows["records"].as_array().unwrap().len(), 3);
        assert_eq!(rows["records"][2]["exit"]["reason"], "runtime-stopped");
    }

    #[test]
    fn rotation_bounds_storage_and_preserves_recent_records() {
        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());
        for sequence in 0..1100 {
            append(
                &context,
                "test-session",
                &json!({"sequence": sequence, "padding": "x".repeat(1024)}),
            )
            .unwrap();
        }
        let rows = read(&context, "test-session", 10).unwrap();
        assert_eq!(rows["records"].as_array().unwrap().len(), 10);
        assert_eq!(rows["records"][9]["sequence"], 1099);
        for name in ["current.jsonl", "previous.jsonl"] {
            assert!(
                fs::metadata(dir.path().join("lifecycle/test-session").join(name))
                    .unwrap()
                    .len()
                    <= SEGMENT_BYTES
            );
        }
    }

    #[test]
    fn concurrent_writers_keep_complete_records() {
        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());
        // Allocate the private root before exercising concurrent appends.
        Journal::lock(&context, "test-session").unwrap();
        std::thread::scope(|scope| {
            for index in 0..24 {
                let context = &context;
                scope.spawn(move || {
                    append(context, "test-session", &json!({"index": index})).unwrap()
                });
            }
        });
        let rows = read(&context, "test-session", 100).unwrap();
        assert_eq!(rows["records"].as_array().unwrap().len(), 24);
    }

    #[test]
    fn refusal_proofs_are_named_and_untrusted_messages_are_not_retained() {
        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());
        for (code, step) in [
            ("session-incarnation-conflict", "incarnation-fence"),
            ("coordination-runtime-unverified", "runtime-stopped-proof"),
        ] {
            let cause = CliError::data(
                code,
                "SECRET-PROMPT-TERMINAL-CANARY",
                Some(json!({"secret": "CAPABILITY-CANARY"})),
            );
            let error = attempt(&context, "test-session", "resume", || {
                Err::<(), _>(cause.clone())
            })
            .unwrap_err();
            assert_eq!(error.message(), cause.message());
            let rows = read(&context, "test-session", 1).unwrap();
            assert_eq!(rows["records"][0]["result"]["code"], code);
            assert_eq!(rows["records"][0]["result"]["proof_step"], step);
            assert!(!rows.to_string().contains("CANARY"));
        }
    }

    #[test]
    fn nested_delete_has_one_success_record_and_linked_store_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());
        attempt(&context, "test-session", "delete", || {
            attempt(&context, "test-session", "delete", || Ok(()))
        })
        .unwrap();
        assert_eq!(
            read(&context, "test-session", 100).unwrap()["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("lifecycle/linked-session"))
            .unwrap();
        assert_eq!(
            append(&context, "linked-session", &json!({}))
                .unwrap_err()
                .code(),
            "lifecycle-journal-unavailable"
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}

/// Allocate through the existing session ID policy before validation so failed
/// creates have a target without changing the normal timestamp/title format.
pub(crate) fn start_id(
    context: &CliContext,
    id: Option<&str>,
    agent: crate::AgentKind,
    title: Option<&str>,
) -> Result<String, CliError> {
    if let Some(id) = id {
        return Ok(id.to_string());
    }
    crate::resolve_session_id(
        context,
        None,
        agent,
        &jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string(),
        title.map(crate::slugify).as_deref(),
    )
}
