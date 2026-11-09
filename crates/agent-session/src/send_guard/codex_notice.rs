//! Prove Codex's logical notice text before submitting or deleting its paste.
use super::{Admission, observation};
use crate::{POST_PASTE_KEY_SETTLE_DELAY, run_status_with_timeout, send_submit};
use std::{process::Command, thread, time::Instant};

impl Admission<'_> {
    pub(super) fn codex_notice_input(&self, text: &str, stage: &str) -> Option<String> {
        let target = format!("{}:0.0", self.record.tmux_session);
        let (pane, _) =
            observation::pane_and_cursor(self.tmux_bin, &target, self.timeout(stage).ok()?)?;
        if let Some(input) = send_submit::exact_composer_input("codex", &pane) {
            return Some(input);
        }
        let input = if send_submit::codex_notice_cursor_path(&pane, text).is_some() {
            text.to_owned()
        } else {
            send_submit::known_multiline_input("codex", &pane, text)?
        };
        let path = send_submit::codex_notice_cursor_path(&pane, &input)?;
        let navigate = |key: &str, count: usize, endpoint: (usize, usize)| -> Option<()> {
            self.identity(stage).ok()?;
            if count > 0 {
                let mut command = Command::new(self.tmux_bin);
                command.args(["send-keys", "-t", &target, "-N", &count.to_string(), key]);
                run_status_with_timeout(command, "tmux send-keys", self.timeout(stage).ok()?)
                    .ok()?;
            }
            let until = (Instant::now() + POST_PASTE_KEY_SETTLE_DELAY).min(self.deadline);
            loop {
                let (current, cursor) = observation::pane_and_cursor(
                    self.tmux_bin,
                    &target,
                    self.timeout(stage).ok()?,
                )?;
                send_submit::codex_notice_cursor_path(&current, &input)?;
                if cursor == endpoint {
                    return Some(());
                }
                if Instant::now() >= until {
                    return None;
                }
                thread::sleep(
                    super::POST_PASTE_OBSERVATION_RETRY_INTERVAL
                        .min(until.saturating_duration_since(Instant::now())),
                );
            }
        };
        // Codex 0.162.1 renders an empty row and a typed blank identically.
        // Reaching both ends proves the length, while each logical row start
        // distinguishes spaces moved between blank rows with the same length.
        navigate("Right", path.characters + 1, path.end)?;
        navigate("Left", path.characters, path.start)?;
        if let Some(end) = path.wrapped_header_end {
            // A hard newline replacing a soft-wrap separator can have the
            // same display and character count. Prove the logical header's
            // end and start before accepting its physical continuation rows.
            navigate("End", 1, end)?;
            navigate("Home", 1, path.start)?;
        }
        for (count, endpoint) in path.rows {
            navigate("Right", count, endpoint)?;
        }
        self.identity(stage).ok()?;
        Some(input)
    }
}
