use super::*;

mod interrupt;

const RECEIPT_FILE: &str = "pane-input-pause.json";
const RECEIPT_SCHEMA: &str = "agent-session.pane-input-pause.v1";

/// Durable proof that this CLI disabled input on one exact pane. It lives in
/// the session directory, so it disappears with the session it belongs to.
#[derive(Deserialize, Serialize)]
struct PauseReceipt {
    schema_version: String,
    id: String,
    launch_id: String,
    tmux_session: String,
    socket_path: String,
    pane_id: String,
}

struct ObservedPane {
    pane: String,
    input_off: bool,
    socket_path: String,
}

fn observe(tmux: &Path, record: &SessionRecord) -> Option<ObservedPane> {
    let mut command = ProcessCommand::new(tmux);
    command.args([
        "display-message",
        "-p",
        "-t",
        &managed_tmux_pane_target(&record.tmux_session),
        "#{pane_id} #{pane_input_off} #{socket_path}",
    ]);
    let output = run_output_with_timeout(command, PANE_OBSERVATION_COMMAND_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    let reply = std::str::from_utf8(&output.stdout)
        .ok()?
        .trim_end_matches('\n');
    let (pane, rest) = reply.split_once(' ')?;
    let (input_off, socket_path) = rest.split_once(' ')?;
    let number = pane.strip_prefix('%')?;
    if number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || socket_path.is_empty()
    {
        return None;
    }
    Some(ObservedPane {
        pane: pane.to_owned(),
        input_off: match input_off {
            "0" => false,
            "1" => true,
            _ => return None,
        },
        socket_path: socket_path.to_owned(),
    })
}

fn select(tmux: &Path, pane: &str, flag: &str) -> Result<(), CliError> {
    let mut command = ProcessCommand::new(tmux);
    command.args(["select-pane", flag, "-t", pane]);
    run_status_with_timeout(
        command,
        "tmux select-pane",
        PANE_OBSERVATION_COMMAND_TIMEOUT,
    )
}

fn runtime_launch_id(record: &SessionRecord) -> &str {
    record
        .runtime
        .as_ref()
        .map_or("", |runtime| runtime.launch_id.as_str())
}

/// Re-enable input on the pane a dead run paused. The caller holds the
/// session record lock, so a receipt found here has no live owner.
///
/// A receipt that no longer matches this session's incarnation, tmux session,
/// server socket and pane id is dropped without touching any pane.
pub(super) fn restore_interrupted(
    tmux: &Path,
    record: &SessionRecord,
    session_dir: &Path,
) -> Result<(), CliError> {
    let path = session_dir.join(RECEIPT_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(session_io_error(
                "pane-input-receipt-read-failed",
                &path,
                error,
            ));
        }
    };
    let unverified = || {
        CliError::runtime(
            "session-status-unknown",
            "running session idleness could not be verified",
            Some(json!({"id": record.id, "state": "unknown"})),
        )
    };
    let receipt: PauseReceipt = serde_json::from_slice(&bytes).map_err(|_| unverified())?;
    let ours = receipt.schema_version == RECEIPT_SCHEMA
        && receipt.id == record.id
        && receipt.launch_id == runtime_launch_id(record)
        && receipt.tmux_session == record.tmux_session;
    let observed = if ours {
        Some(observe(tmux, record).ok_or_else(unverified)?)
    } else {
        None
    };
    if let Some(observed) = observed.filter(|observed| {
        observed.pane == receipt.pane_id && observed.socket_path == receipt.socket_path
    }) && observed.input_off
    {
        select(tmux, &receipt.pane_id, "-e").map_err(|_| unverified())?;
        let confirmed = observe(tmux, record).ok_or_else(unverified)?;
        if confirmed.pane != receipt.pane_id || confirmed.input_off {
            return Err(unverified());
        }
    }
    fs::remove_file(&path).or_else(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(session_io_error(
                "pane-input-receipt-remove-failed",
                &path,
                error,
            ))
        }
    })
}

pub(super) struct PausedPaneInput<'a> {
    tmux: &'a Path,
    pane: String,
    receipt: PathBuf,
    armed: bool,
    _interrupt: Option<interrupt::Watch>,
}

impl<'a> PausedPaneInput<'a> {
    pub(super) fn pause(
        tmux: &'a Path,
        record: &SessionRecord,
        session_dir: &Path,
    ) -> Option<Self> {
        let observed = observe(tmux, record)?;
        // Do not claim a pane whose input was already disabled.
        if observed.input_off {
            return None;
        }
        let pane = observed.pane.as_str();
        // An after-send-keys hook can yield between the shutdown key and the
        // re-pause, letting the attached client type in the gap.
        if session_profile_graceful_shutdown(record).ok()?.is_some() {
            let mut command = ProcessCommand::new(tmux);
            command.args(["show-options", "-AHv", "-t", pane, "after-send-keys"]);
            let output = run_output_with_timeout(command, PANE_OBSERVATION_COMMAND_TIMEOUT).ok()?;
            if !output.status.success() || !output.stdout.is_empty() {
                return None;
            }
        }
        // Persist ownership before the pause so a run that dies afterwards
        // leaves the proof needed to restore exactly this pane.
        let receipt_path = session_dir.join(RECEIPT_FILE);
        let receipt = PauseReceipt {
            schema_version: RECEIPT_SCHEMA.to_owned(),
            id: record.id.clone(),
            launch_id: runtime_launch_id(record).to_owned(),
            tmux_session: record.tmux_session.clone(),
            socket_path: observed.socket_path.clone(),
            pane_id: observed.pane.clone(),
        };
        write_private_file(&receipt_path, &serde_json::to_vec(&receipt).ok()?).ok()?;
        let mut guard = Self {
            tmux,
            pane: observed.pane,
            receipt: receipt_path.clone(),
            armed: true,
            _interrupt: None,
        };
        let (interrupted_tmux, interrupted_pane) = (tmux.to_path_buf(), guard.pane.clone());
        guard._interrupt = interrupt::Watch::install(move || {
            let _ = restore(&interrupted_tmux, &interrupted_pane, &receipt_path);
        });
        // Arm before issuing the command in case it takes effect but times out.
        select(tmux, &guard.pane, "-d").ok()?;
        Some(guard)
    }

    pub(super) fn pane(&self) -> &str {
        &self.pane
    }

    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

fn restore(tmux: &Path, pane: &str, receipt: &Path) -> Result<(), CliError> {
    select(tmux, pane, "-e")?;
    // The receipt is cleared only after tmux confirmed the restore.
    let _ = fs::remove_file(receipt);
    Ok(())
}

impl Drop for PausedPaneInput<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = restore(self.tmux, &self.pane, &self.receipt)
        {
            eprintln!(
                "error: could not restore tmux pane input ({})",
                error.code()
            );
        }
    }
}

impl TmuxRuntimeIdentity {
    pub(crate) fn graceful_send_command(&self, paused_pane: Option<&str>) -> String {
        let pane = &self.pane_id;
        match paused_pane {
            // tmux also drops send-keys while the pane is disabled. Keep the
            // enable, send and disable together in one synchronous command list.
            Some(paused) if paused == pane => {
                format!(
                    "select-pane -e -t {pane} ; send-keys -t {pane} C-c ; select-pane -d -t {pane}"
                )
            }
            Some(_) => format!("display-message -p {TMUX_RUNTIME_IDENTITY_CHANGED_OUTPUT}"),
            None => format!("send-keys -t {pane} C-c"),
        }
    }
}
