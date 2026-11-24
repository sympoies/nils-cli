//! Fence native provider requests independently of turn/start authorization.
use super::*;
use std::io;

const CAPABILITY_FILE: &str = ".codex-app-server-idle-retirement";
const CAPABILITY_VERSION: &str = "agent-session.codex-idle-retirement.v2";
const REQUEST_LOCK: &str = ".codex-app-server-retirement-requests.lock";

fn capability_path(context: &CliContext, record: &SessionRecord) -> PathBuf {
    crate::session_dir(context, &record.id).join(CAPABILITY_FILE)
}

pub(super) fn advertise(context: &CliContext, record: &SessionRecord) -> Option<ManualInputMarker> {
    write_runtime_process_marker(
        capability_path(context, record),
        record,
        CAPABILITY_VERSION,
        PROXY_CAPABILITY_TTL,
    )
    .ok()
}

pub(crate) fn supported(context: &CliContext, record: &SessionRecord) -> bool {
    runtime_is_supported(record)
        && persisted_runtime_paths(context, record).is_ok()
        && live_marker(
            &capability_path(context, record),
            record,
            CAPABILITY_VERSION,
        )
        .is_some()
}

pub(super) fn live_marker(
    path: &Path,
    record: &SessionRecord,
    version: &str,
) -> Option<RuntimeProcessMarker> {
    let mut file = fs::File::open(path).ok()?;
    let (own, current) = (file.metadata().ok()?, fs::metadata(path).ok()?);
    if own.dev() != current.dev() || own.ino() != current.ino() {
        return None;
    }
    // A live proxy holds a shared lock for the advertised lifetime.
    if try_lock(&file, libc::LOCK_EX).ok()? {
        return None;
    }
    read_runtime_process_marker_file(&mut file).filter(|marker| {
        valid_runtime_process_marker(marker, record, version, PROXY_CAPABILITY_TTL)
    })
}

pub(crate) fn probe_requests(
    context: &CliContext,
    record: &SessionRecord,
) -> io::Result<Option<fs::File>> {
    let fence = open_request_lock(context, record)?;
    Ok(try_lock(&fence, libc::LOCK_EX)?.then_some(fence))
}

fn open_request_lock(context: &CliContext, record: &SessionRecord) -> io::Result<fs::File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(crate::session_dir(context, &record.id).join(REQUEST_LOCK))
}

fn try_lock(file: &fs::File, mode: libc::c_int) -> io::Result<bool> {
    // SAFETY: flock borrows the descriptor owned by file.
    if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error)
    }
}

// Codex app-server v2 (0.171.0). Initialization and these read-only probes
// can pass during retirement. Turn/start has its own gate. Other RPC requests
// can change provider state, so they hold a lease through the matching reply.
fn requires_lease(method: &str) -> bool {
    !matches!(
        method,
        "initialize"
            | "turn/start"
            | "thread/list"
            | "thread/loaded/list"
            | "thread/read"
            | "thread/turns/list"
            | "thread/items/list"
            | "thread/goal/get"
            | "model/list"
            | "account/read"
    )
}

#[derive(Default)]
pub(super) struct Requests(BTreeMap<String, fs::File>);

impl Requests {
    pub(super) async fn authorize(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
        bootstrap: &mut FreshBootstrap,
        value: &Value,
    ) -> Result<MutationAuthorization, TuiMutationRejection> {
        let authorization =
            cancel_before_tui_mutation_detailed(context, record, bootstrap, value).await?;
        if !value
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(requires_lease)
            || !runtime_is_supported(record)
        {
            return Ok(authorization);
        }
        let Some(key) = value.get("id").and_then(json_id_key) else {
            return Ok(authorization);
        };
        let fence = open_request_lock(context, record)
            .map_err(|_| TuiMutationRejection::TurnGateOpenFailed)?;
        let deadline = tokio::time::Instant::now() + CONTROL_SUBMIT_TOTAL_TIMEOUT;
        let mut waited = false;
        loop {
            if try_lock(&fence, libc::LOCK_SH)
                .map_err(|_| TuiMutationRejection::TurnGateOpenFailed)?
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(TuiMutationRejection::TurnGateBusy);
            }
            waited = true;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // A concurrent delete can complete while a request waits. It must
        // never forward a buffered request on a retired proxy connection.
        if waited
            && !crate::load_session_record(context, &record.id)
                .is_ok_and(|current| crate::ensure_same_session_identity(record, &current).is_ok())
        {
            return Err(TuiMutationRejection::TurnGateBusy);
        }
        self.0.insert(key, fence);
        Ok(authorization)
    }

    pub(super) fn observe_response(&mut self, value: &Value) {
        if let Some(key) = json_rpc_response_id_key(value) {
            self.0.remove(&key);
        }
    }
}
