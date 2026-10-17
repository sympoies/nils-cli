//! Removal of text a guarded send pasted and then refused to submit.
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Admission, capture_styled};
use crate::{CliError, run_status_with_timeout, send_submit};

/// Time given to the observations and keys of one cleanup, independent of the
/// admission budget the refusal may already have spent.
const CLEAR_BUDGET: Duration = Duration::from_millis(2500);
const CLEAR_POLL: Duration = Duration::from_millis(100);
const CLEAR_SETTLE: Duration = Duration::from_millis(300);

impl Admission<'_> {
    /// Report a failure that happened after the send pasted `text`. The text is
    /// removed only while the composer still holds exactly it, so a draft, a
    /// submission or an unreadable composer is never touched. The error says
    /// whether the input was proven empty or may still hold the text.
    pub(crate) fn report_pasted_input(&self, mut error: CliError, text: &str) -> CliError {
        let remaining = self.clear_pasted(text);
        let cleared = remaining.as_deref() == Some("");
        let remaining_input = match remaining.as_deref() {
            Some("") => "empty",
            Some(input) if send_submit::composer_holds(&self.record.agent, input, text) => {
                "pasted_text"
            }
            Some(_) => "different_text",
            None => "unobservable",
        };
        let details = error.0.details.get_or_insert_with(|| json!({}));
        if !details.is_object() {
            *details = json!({});
        }
        details["pasted"] = Value::Bool(true);
        details["cleared"] = Value::Bool(cleared);
        details["remaining_input"] = json!(remaining_input);
        if let Some(input) = remaining {
            details["remaining_characters"] = json!(input.chars().count());
            details["pasted_text_present"] = json!(!input.is_empty() && input.contains(text));
        }
        error.0.message = format!(
            "{}; {}",
            error.0.message,
            match remaining_input {
                "empty" => "the session input no longer holds the pasted text",
                "pasted_text" => "the session input still holds the pasted text",
                "different_text" =>
                    "different composer text was preserved; the pasted text may remain",
                _ =>
                    "the composer is unobservable; the pasted text may remain in the session input",
            }
        );
        error
    }

    fn clear_pasted(&self, text: &str) -> Option<String> {
        let cleanup = Admission {
            deadline: Instant::now() + CLEAR_BUDGET,
            context: self.context,
            record: self.record,
            args: self.args,
            tmux_bin: self.tmux_bin,
            verify_composer: self.verify_composer,
        };
        cleanup.clear(text)
    }

    fn clear(&self, text: &str) -> Option<String> {
        let target = format!("{}:0.0", self.record.tmux_session);
        let observe = |stage: &str| {
            let pane = capture_styled(self.tmux_bin, &target, self.timeout(stage).ok()?)?;
            send_submit::cleanup_composer_input(&self.record.agent, &pane)
        };
        self.identity("cleanup").ok()?;
        let mut input = observe("cleanup")?;
        if input.is_empty() {
            // A paste can still be rendering: only a second empty read proves it gone.
            self.timeout("cleanup").ok()?;
            thread::sleep(CLEAR_SETTLE);
            input = observe("cleanup")?;
            if input.is_empty() {
                return Some(input);
            }
        }
        if !send_submit::composer_holds(&self.record.agent, &input, text) {
            return Some(input);
        }
        // One tmux command keeps long wrapped pastes inside the cleanup budget.
        // As before, one Backspace per character cannot delete past the start.
        let mut command = std::process::Command::new(self.tmux_bin);
        command.args([
            "send-keys",
            "-t",
            &target,
            "-N",
            &text.chars().count().to_string(),
            "BSpace",
        ]);
        let timeout = self.timeout("cleanup").ok()?;
        let sent = run_status_with_timeout(command, "tmux send-keys", timeout).is_ok();
        // Sending keys invalidates the earlier observation, even on timeout.
        input = observe("cleanup")?;
        loop {
            if input.is_empty() || !sent || self.timeout("cleanup").is_err() {
                return Some(input);
            }
            thread::sleep(CLEAR_POLL);
            let Some(current) = observe("cleanup") else {
                return self.timeout("cleanup").is_err().then_some(input);
            };
            input = current;
        }
    }
}
