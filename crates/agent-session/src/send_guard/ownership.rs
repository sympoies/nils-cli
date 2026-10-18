//! Observe the paste before a later edit can be mistaken for its suffix.
use super::{Admission, capture_styled};
use crate::{POST_PASTE_KEY_SETTLE_DELAY, send_submit};
use std::{
    thread,
    time::{Duration, Instant},
};

impl Admission<'_> {
    pub(crate) fn settle_paste(&self, text: &str) -> bool {
        let until = Instant::now() + POST_PASTE_KEY_SETTLE_DELAY;
        if self.record.agent != "claude" || !self.args.require_empty_composer {
            thread::sleep(POST_PASTE_KEY_SETTLE_DELAY);
            return false;
        }
        let target = format!("{}:0.0", self.record.tmux_session);
        let mut seen = false;
        while Instant::now() < until {
            let Some(timeout) = self.timeout("submit").ok() else {
                break;
            };
            let input = capture_styled(
                self.tmux_bin,
                &target,
                timeout.min(until.saturating_duration_since(Instant::now())),
            )
            .and_then(|pane| {
                send_submit::cleanup_composer_input(&self.record.agent, &pane, Some(text))
            });
            if let Some(input) = input {
                if send_submit::composer_holds(&self.record.agent, &input, text) {
                    seen = true;
                    break;
                }
                if !input.is_empty() && !text.starts_with(&input) {
                    // A mixed/changed input is not evidence of paste ownership.
                    break;
                }
            }
            thread::sleep(
                Duration::from_millis(50).min(until.saturating_duration_since(Instant::now())),
            );
        }
        thread::sleep(until.saturating_duration_since(Instant::now()));
        seen
    }
}
