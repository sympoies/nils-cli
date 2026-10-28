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
    /// removed only when its ownership is observable: an exact match or a
    /// countable-cell prefix of a line admitted with an empty composer. An appended
    /// draft is preserved. Every outcome includes a fresh input observation.
    pub(crate) fn report_pasted_input(
        &self,
        mut error: CliError,
        text: &str,
        observed_paste: bool,
    ) -> CliError {
        let (remaining, cleared) = self.clear_pasted(text, observed_paste);
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
        details["paste_observed_before_conflict"] = Value::Bool(observed_paste);
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
                _ if cleared =>
                    "the observed paste prefix was removed; remaining input matches the observed suffix",
                "pasted_text" => "the session input still holds the pasted text",
                "different_text" if !observed_paste =>
                    "composer text was left unchanged; paste ownership was not verified",
                "different_text" =>
                    "different composer text was preserved; removal of the pasted prefix was not verified",
                _ =>
                    "the composer is unobservable; the pasted text may remain in the session input",
            }
        );
        error
    }

    fn clear_pasted(&self, text: &str, observed_paste: bool) -> (Option<String>, bool) {
        let cleanup = Admission {
            deadline: Instant::now() + CLEAR_BUDGET,
            context: self.context,
            record: self.record,
            args: self.args,
            tmux_bin: self.tmux_bin,
            verify_composer: self.verify_composer,
        };
        cleanup
            .clear(text, observed_paste)
            .map_or((None, false), |(input, restored)| (Some(input), restored))
    }

    fn clear(&self, text: &str, observed_paste: bool) -> Option<(String, bool)> {
        let target = format!("{}:0.0", self.record.tmux_session);
        let observe = |stage: &str| {
            let pane = capture_styled(self.tmux_bin, &target, self.timeout(stage).ok()?)?;
            send_submit::cleanup_composer_input(&self.record.agent, &pane, Some(text))
        };
        self.identity("cleanup").ok()?;
        let mut input = observe("cleanup")?;
        if input.is_empty() {
            // A paste can still be rendering: only a second empty read proves it gone.
            self.timeout("cleanup").ok()?;
            thread::sleep(CLEAR_SETTLE);
            input = observe("cleanup")?;
            if input.is_empty() {
                return Some((input, true));
            }
        }
        let exact = send_submit::composer_holds(&self.record.agent, &input, text);
        let suffix = input.strip_prefix(text);
        let prefix = observed_paste
            && self.record.agent == "claude"
            && self.args.require_empty_composer
            && suffix.is_some_and(|suffix| !suffix.is_empty())
            && send_submit::cleanup_cursor_cells(&input).is_some();
        if !exact && !prefix {
            return Some((input, false));
        }
        let expected = if prefix {
            suffix?.to_string()
        } else {
            String::new()
        };
        // Claude cursor-counted cleanup removes a prefix from the logical line start,
        // including pasted-only input. Verify both the start and the endpoint
        // reached by walking exactly the known prefix length before deleting.
        // This also verifies spaces omitted by the word-wrapped rendering.
        let single_row_exact =
            capture_styled(self.tmux_bin, &target, self.timeout("cleanup").ok()?)
                .and_then(|pane| send_submit::exact_composer_input(&self.record.agent, &pane))
                .is_some_and(|current| {
                    send_submit::composer_holds(&self.record.agent, &current, text)
                });
        let forward = !single_row_exact
            && self.record.agent == "claude"
            && send_submit::cleanup_cursor_cells(&input).is_some();
        if forward {
            let mut home = std::process::Command::new(self.tmux_bin);
            home.args(["send-keys", "-t", &target, "C-a"]);
            run_status_with_timeout(home, "tmux send-keys", self.timeout("cleanup").ok()?).ok()?;
            let mut endpoint = None;
            loop {
                let (pane, coordinates) = super::observation::pane_and_cursor(
                    self.tmux_bin,
                    &target,
                    self.timeout("cleanup").ok()?,
                )?;
                let Some(current) =
                    send_submit::cleanup_composer_input(&self.record.agent, &pane, Some(text))
                else {
                    if self.deadline.saturating_duration_since(Instant::now()) <= CLEAR_POLL {
                        return None;
                    }
                    thread::sleep(CLEAR_POLL);
                    continue;
                };
                if current != input {
                    return Some((current, false));
                }
                let at_target = match endpoint {
                    None => {
                        send_submit::cleanup_cursor_at_start(&pane, coordinates.0, coordinates.1)
                    }
                    Some(end) => end == (coordinates.0, coordinates.1),
                };
                if at_target {
                    if endpoint.is_some() {
                        break;
                    }
                    endpoint = Some(send_submit::cleanup_prefix_end(&pane, text)?);
                    let mut right = std::process::Command::new(self.tmux_bin);
                    right.args([
                        "send-keys",
                        "-t",
                        &target,
                        "-N",
                        &text.chars().count().to_string(),
                        "Right",
                    ]);
                    run_status_with_timeout(right, "tmux send-keys", self.timeout("cleanup").ok()?)
                        .ok()?;
                    continue;
                }
                if self.deadline.saturating_duration_since(Instant::now()) <= CLEAR_POLL {
                    return None;
                }
                thread::sleep(CLEAR_POLL);
            }
        }
        self.identity("cleanup").ok()?;
        // One repeated-key command keeps long pastes inside the cleanup budget.
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
        // Rendering can be transiently unobservable while repeated keys are
        // processed. Poll without issuing more keys; only a post-key read can
        // establish restoration, including when tmux timed out.
        let mut last_observed = None;
        loop {
            if self.timeout("cleanup").is_err() {
                return last_observed.map(|input| (input, false));
            }
            let remaining = observe("cleanup");
            // A timeout cannot invalidate an earlier post-key observation:
            // no further keys were issued. Other unreadable captures do.
            if remaining.is_none() && self.timeout("cleanup").is_err() {
                return last_observed.map(|input| (input, false));
            }
            if remaining.as_deref() == Some(expected.as_str()) {
                return Some((remaining?, true));
            }
            if !sent || self.deadline.saturating_duration_since(Instant::now()) <= CLEAR_POLL {
                return remaining.map(|input| (input, false));
            }
            last_observed = remaining;
            thread::sleep(CLEAR_POLL);
        }
    }
}
