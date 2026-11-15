//! One synchronous hook owns each socket; answers never enter a terminal.
use super::{AnswerRequest, Wait, refusal};
use crate::{
    CliContext, CliError, SessionRecord, activity::claude_attention::with_permission_call,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

// Claude Code 2.1.317 command hooks default to 600s. Leave time for the
// dispatcher to return output; expiration never grants permission.
const WAIT_BOUND: Duration = Duration::from_secs(540);
const IO_BOUND: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    request_id: String,
    call_id: String,
    runtime_id: String,
    attention_id: String,
    socket: PathBuf,
}

struct Registration(PathBuf);
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn directory(context: &CliContext, id: &str) -> PathBuf {
    crate::session_dir(context, id).join("permissions")
}
fn lock_path(directory: &Path, call_id: &str) -> PathBuf {
    let digest = Sha256::digest(call_id.as_bytes());
    let digest: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    directory.join(format!("{digest}.lock"))
}
fn locked(directory: &Path, call_id: &str) -> Result<fs::File, CliError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(lock_path(directory, call_id))
        .map_err(|_| refusal("permission-hook-unavailable"))?;
    // SAFETY: the owned descriptor is valid; the file retains the lease.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(refusal("permission-hook-already-waiting"));
    }
    Ok(file)
}
fn active(directory: &Path, call_id: &str) -> bool {
    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(lock_path(directory, call_id))
    else {
        return false;
    };
    // SAFETY: the descriptor is valid. Only a conflicting lease proves a waiter.
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) != 0
}
fn read(path: &Path) -> Result<Pending, CliError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| refusal("permission-request-gone"))?;
    let metadata = file
        .metadata()
        .map_err(|_| refusal("permission-request-gone"))?;
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(refusal("permission-hook-unavailable"));
    }
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| refusal("permission-hook-unavailable"))?;
    if bytes.len() > 4096 {
        return Err(refusal("permission-hook-unavailable"));
    }
    serde_json::from_slice(&bytes).map_err(|_| refusal("permission-hook-unavailable"))
}
fn current(context: &CliContext, id: &str, runtime: &str) -> Result<SessionRecord, CliError> {
    let record = crate::load_session_record(context, id)?;
    if record.agent != "claude"
        || record
            .runtime
            .as_ref()
            .is_none_or(|r| r.kind != "tmux" || r.launch_id != runtime)
    {
        return Err(refusal("permission-runtime-mismatch"));
    }
    crate::restart::ensure_not_fenced(context, &record)?;
    Ok(record)
}
fn check<T>(
    context: &CliContext,
    id: &str,
    pending: &Pending,
    operation: impl FnOnce() -> Result<T, CliError>,
) -> Result<T, CliError> {
    let _record_lock = crate::acquire_session_record_lock(context, id)?;
    let record = current(context, id, &pending.runtime_id)?;
    with_permission_call(context, &record, &pending.attention_id, |call| {
        if call != pending.call_id {
            return Err(refusal("permission-call-mismatch"));
        }
        operation()
    })
}

pub(super) fn list(context: &CliContext, id: &str) -> Result<Vec<Value>, CliError> {
    let record = crate::load_session_record(context, id)?;
    super::version::check(&record)?;
    let directory = directory(context, id);
    let mut requests = Vec::new();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(requests),
        Err(_) => return Err(refusal("permission-hook-unavailable")),
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        if let Ok(pending) = read(&entry.path())
            && active(&directory, &pending.call_id)
            && check(context, id, &pending, || Ok(())).is_ok()
        {
            requests.push(json!({"request_id": pending.request_id, "call_id": pending.call_id, "runtime_id": pending.runtime_id}));
        }
    }
    requests.sort_by(|a, b| a["request_id"].as_str().cmp(&b["request_id"].as_str()));
    Ok(requests)
}

pub(super) fn answer(
    context: &CliContext,
    id: &str,
    request: AnswerRequest,
) -> Result<Value, CliError> {
    crate::load_session_record(context, id)?;
    let uuid = uuid::Uuid::parse_str(&request.request_id)
        .map_err(|_| refusal("permission-request-gone"))?;
    if uuid.to_string() != request.request_id {
        return Err(refusal("permission-request-gone"));
    }
    let pending = read(&directory(context, id).join(format!("{uuid}.json")))?;
    if pending.request_id != request.request_id {
        return Err(refusal("permission-request-gone"));
    }
    if pending.call_id != request.call_id {
        return Err(refusal("permission-call-mismatch"));
    }
    // Preflight gives precise stale reasons; the waiter repeats it under both
    // locks immediately before emitting its own hook's decision.
    check(context, id, &pending, || Ok(()))?;
    let mut stream =
        UnixStream::connect(&pending.socket).map_err(|_| refusal("permission-request-gone"))?;
    stream
        .set_read_timeout(Some(IO_BOUND))
        .map_err(|_| refusal("permission-hook-unavailable"))?;
    stream
        .set_write_timeout(Some(IO_BOUND))
        .map_err(|_| refusal("permission-hook-unavailable"))?;
    let bytes = serde_json::to_vec(&request).map_err(|_| refusal("permission-hook-unavailable"))?;
    stream
        .write_all(&bytes)
        .map_err(|_| refusal("permission-delivery-unknown"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|_| refusal("permission-delivery-unknown"))?;
    let mut bytes = Vec::new();
    (&mut stream)
        .take(1025)
        .read_to_end(&mut bytes)
        .map_err(|_| refusal("permission-delivery-unknown"))?;
    if bytes.len() > 1024 {
        return Err(refusal("permission-delivery-unknown"));
    }
    let response: Value =
        serde_json::from_slice(&bytes).map_err(|_| refusal("permission-delivery-unknown"))?;
    if response["outcome"] != "delivered_to_hook" {
        return Err(refusal("permission-request-gone"));
    }
    Ok(
        json!({"request_id": request.request_id, "call_id": request.call_id, "decision": request.decision, "outcome": "delivered_to_hook"}),
    )
}

pub(super) fn wait(context: &CliContext, args: Wait) -> Result<(), CliError> {
    let directory = directory(context, &args.id);
    let (pending, _lease, _socket_dir, listener, _registration) = {
        let _record_lock = crate::acquire_session_record_lock(context, &args.id)?;
        let record = current(context, &args.id, &args.runtime_id)?;
        super::version::check(&record)?;
        with_permission_call(context, &record, &args.attention_id, |call| {
            fs::create_dir_all(&directory).map_err(|_| refusal("permission-hook-unavailable"))?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| refusal("permission-hook-unavailable"))?;
            let lease = locked(&directory, call)?;
            // A private short path also works when the session state path is
            // longer than the Unix socket pathname limit (including macOS).
            let socket_dir = tempfile::Builder::new()
                .prefix("agent-permission-")
                .tempdir_in("/tmp")
                .map_err(|_| refusal("permission-hook-unavailable"))?;
            let socket = socket_dir.path().join("s");
            let listener =
                UnixListener::bind(&socket).map_err(|_| refusal("permission-hook-unavailable"))?;
            fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
                .map_err(|_| refusal("permission-hook-unavailable"))?;
            listener
                .set_nonblocking(true)
                .map_err(|_| refusal("permission-hook-unavailable"))?;
            let pending = Pending {
                request_id: uuid::Uuid::new_v4().to_string(),
                call_id: call.to_string(),
                runtime_id: args.runtime_id.clone(),
                attention_id: args.attention_id.clone(),
                socket,
            };
            let path = directory.join(format!("{}.json", pending.request_id));
            crate::write_private_file(
                &path,
                &serde_json::to_vec(&pending)
                    .map_err(|_| refusal("permission-hook-unavailable"))?,
            )?;
            Ok((pending, lease, socket_dir, listener, Registration(path)))
        })?
    };
    // SAFETY: getppid has no preconditions. Orphaned relays must not survive
    // their dispatcher and leave an answerable request behind.
    let parent = unsafe { libc::getppid() };
    let deadline = Instant::now() + WAIT_BOUND;
    loop {
        if Instant::now() >= deadline {
            return Err(refusal("permission-hook-expired"));
        }
        if unsafe { libc::getppid() } != parent {
            return Err(refusal("permission-request-gone"));
        }
        check(context, &args.id, &pending, || Ok(()))?;
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_read_timeout(Some(IO_BOUND))
                    .map_err(|_| refusal("permission-hook-unavailable"))?;
                stream
                    .set_write_timeout(Some(IO_BOUND))
                    .map_err(|_| refusal("permission-hook-unavailable"))?;
                let mut bytes = Vec::new();
                if (&mut stream).take(4097).read_to_end(&mut bytes).is_err() || bytes.len() > 4096 {
                    continue;
                }
                let Ok(request) = serde_json::from_slice::<AnswerRequest>(&bytes) else {
                    continue;
                };
                if request.request_id != pending.request_id || request.call_id != pending.call_id {
                    continue;
                }
                deliver(
                    context,
                    &args.id,
                    &pending,
                    &request,
                    deadline,
                    &mut stream,
                    &mut std::io::stdout().lock(),
                )?;
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100))
            }
            Err(_) => return Err(refusal("permission-hook-unavailable")),
        }
    }
}

// The received answer crosses the final identity/expiry fence before any output
// or acknowledgement. A writer argument lets fixtures observe this boundary
// without adding a provider protocol message or runtime testing switch.
fn deliver(
    context: &CliContext,
    id: &str,
    pending: &Pending,
    request: &AnswerRequest,
    deadline: Instant,
    stream: &mut UnixStream,
    output: &mut impl Write,
) -> Result<(), CliError> {
    check(context, id, pending, || {
        if Instant::now() >= deadline {
            return Err(refusal("permission-hook-expired"));
        }
        let decision = json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": request.decision}}});
        serde_json::to_writer(&mut *output, &decision)
            .map_err(|_| refusal("permission-delivery-unknown"))?;
        output
            .flush()
            .map_err(|_| refusal("permission-delivery-unknown"))?;
        Ok(())
    })?;
    // The decision is already emitted; losing this acknowledgement is uncertain
    // delivery, never permission to retry another hook.
    let _ = stream.write_all(b"{\"outcome\":\"delivered_to_hook\"}");
    Ok(())
}

#[cfg(test)]
mod tests;
