use super::*;

pub(super) struct PausedPaneInput<'a> {
    tmux: &'a Path,
    pane: String,
    armed: bool,
}

impl<'a> PausedPaneInput<'a> {
    pub(super) fn pause(tmux: &'a Path, record: &SessionRecord) -> Option<Self> {
        let mut command = ProcessCommand::new(tmux);
        command.args([
            "display-message",
            "-p",
            "-t",
            &managed_tmux_pane_target(&record.tmux_session),
            "#{pane_id} #{pane_input_off}",
        ]);
        let output = run_output_with_timeout(command, PANE_OBSERVATION_COMMAND_TIMEOUT).ok()?;
        if !output.status.success() {
            return None;
        }
        let reply = std::str::from_utf8(&output.stdout).ok()?;
        // Do not claim a pane whose input was already disabled.
        let (pane, "0") = reply.trim().split_once(' ')? else {
            return None;
        };
        let number = pane.strip_prefix('%')?;
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
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
        let guard = Self {
            tmux,
            pane: pane.to_owned(),
            armed: true,
        };
        // Arm before issuing the command in case it takes effect but times out.
        guard.select("-d").ok()?;
        Some(guard)
    }

    pub(super) fn pane(&self) -> &str {
        &self.pane
    }

    pub(super) fn disarm(mut self) {
        self.armed = false;
    }

    fn select(&self, flag: &str) -> Result<(), CliError> {
        let mut command = ProcessCommand::new(self.tmux);
        command.args(["select-pane", flag, "-t", &self.pane]);
        run_status_with_timeout(
            command,
            "tmux select-pane",
            PANE_OBSERVATION_COMMAND_TIMEOUT,
        )
    }
}

impl Drop for PausedPaneInput<'_> {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = self.select("-e")
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
