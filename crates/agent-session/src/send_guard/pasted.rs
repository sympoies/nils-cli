//! Prove a complete wrapped paste before Enter, including hidden separators.
use super::{Admission, observation};
use crate::{run_status_with_timeout, send_submit};
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

impl Admission<'_> {
    pub(super) fn pasted_composer_input(
        &self,
        pane: &str,
        pasted: Option<&str>,
        stage: &str,
    ) -> Option<String> {
        if let Some(input) = send_submit::exact_composer_input(&self.record.agent, pane) {
            return Some(input);
        }
        // Only a send admitted against an empty composer owns the known paste.
        // Cleanup's general joined-row fallback is not submit evidence.
        if self.record.agent != "claude" || !self.args.require_empty_composer {
            return None;
        }
        let text = pasted?;
        let end = send_submit::cleanup_prefix_end(pane, text)?;
        let input = send_submit::cleanup_composer_input("claude", pane, Some(text))?;
        if input != text {
            return None;
        }
        let target = format!("{}:0.0", self.record.tmux_session);
        // A visual layout may omit or pad spaces. Check the logical length:
        // N-1 characters must stop one cell before the known endpoint, and one
        // more must reach it. This rejects both extra and missing separators.
        // A matched final row contains a nonempty paste remainder, so its
        // endpoint is past column 2 and the preceding cell is defined.
        for (key, count, endpoint) in [
            ("C-a", 1, None),
            (
                "Right",
                text.len().checked_sub(1)?,
                Some((end.0 - 1, end.1)),
            ),
            ("Right", 1, Some(end)),
        ] {
            self.identity(stage).ok()?;
            let mut command = Command::new(self.tmux_bin);
            command.args(["send-keys", "-t", &target, "-N", &count.to_string(), key]);
            run_status_with_timeout(command, "tmux send-keys", self.timeout(stage).ok()?).ok()?;
            let until = Instant::now() + crate::POST_PASTE_KEY_SETTLE_DELAY;
            loop {
                let (current, cursor) = observation::pane_and_cursor(
                    self.tmux_bin,
                    &target,
                    self.timeout(stage).ok()?,
                )?;
                if send_submit::cleanup_composer_input("claude", &current, Some(text))? != input {
                    return None;
                }
                let reached = endpoint.map_or_else(
                    || send_submit::cleanup_cursor_at_start(&current, cursor.0, cursor.1),
                    |expected| cursor == expected,
                );
                if reached {
                    break;
                }
                if Instant::now() >= until {
                    return None;
                }
                thread::sleep(Duration::from_millis(50).min(self.timeout(stage).ok()?));
            }
        }
        self.identity(stage).ok()?;
        Some(input)
    }
}
