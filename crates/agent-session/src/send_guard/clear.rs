//! Removal of text a guarded send pasted and then refused to submit.
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Admission, capture_styled};
use crate::{CliError, SpecialKey, send_submit, send_tmux_key_with_timeout};

/// Time given to the observations and keys of one cleanup, independent of the
/// admission budget the refusal may already have spent.
const CLEAR_BUDGET: Duration = Duration::from_millis(2500);
const CLEAR_POLL: Duration = Duration::from_millis(100);

impl Admission<'_> {
    /// Report a failure that happened after the send pasted `text`. The text is
    /// removed only while the composer still holds exactly it, so a draft, a
    /// submission or an unreadable composer is never touched. The error says
    /// whether the input was proven empty or may still hold the text.
    pub(crate) fn report_pasted_input(&self, mut error: CliError, text: &str) -> CliError {
        let cleared = self.clear_pasted(text);
        let details = error.0.details.get_or_insert_with(|| json!({}));
        if !details.is_object() {
            *details = json!({});
        }
        details["pasted"] = Value::Bool(true);
        details["cleared"] = Value::Bool(cleared);
        error.0.message = format!(
            "{}; {}",
            error.0.message,
            if cleared {
                "the session input no longer holds the pasted text"
            } else {
                "the pasted text may remain in the session input"
            }
        );
        error
    }

    fn clear_pasted(&self, text: &str) -> bool {
        let cleanup = Admission {
            deadline: Instant::now() + CLEAR_BUDGET,
            context: self.context,
            record: self.record,
            args: self.args,
            tmux_bin: self.tmux_bin,
            verify_composer: self.verify_composer,
        };
        cleanup.clear(text).is_some()
    }

    fn clear(&self, text: &str) -> Option<()> {
        let target = format!("{}:0.0", self.record.tmux_session);
        let observe = |stage: &str| {
            let pane = capture_styled(self.tmux_bin, &target, self.timeout(stage).ok()?)?;
            send_submit::exact_composer_input(&self.record.agent, &pane)
        };
        self.identity("cleanup").ok()?;
        let input = observe("cleanup")?;
        if input.is_empty() {
            return Some(());
        }
        if !send_submit::composer_holds(&self.record.agent, &input, text) {
            return None;
        }
        // One key removes one character, and a key past the start is harmless.
        for _ in text.chars() {
            let timeout = self.timeout("cleanup").ok()?;
            send_tmux_key_with_timeout(self.tmux_bin, &target, SpecialKey::Backspace, timeout)
                .ok()?;
        }
        loop {
            if observe("cleanup")?.is_empty() {
                return Some(());
            }
            self.timeout("cleanup").ok()?;
            thread::sleep(CLEAR_POLL);
        }
    }
}
