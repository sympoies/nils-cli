//! Prove a complete wrapped paste before Enter, including hidden separators.
use super::{Admission, observation};
use crate::{run_status_with_timeout, send_submit};
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, PartialEq, Eq)]
enum WrappedProofObservation {
    Exact(String),
    Mismatch(String),
    Unobservable,
}

fn classify_wrapped_proof(expected: &str, observed: Option<String>) -> WrappedProofObservation {
    match observed {
        Some(input) if input == expected => WrappedProofObservation::Exact(input),
        Some(input) => WrappedProofObservation::Mismatch(input),
        _ => WrappedProofObservation::Unobservable,
    }
}

impl Admission<'_> {
    pub(super) fn pasted_composer_input(
        &self,
        pane: &str,
        pasted: Option<&str>,
        stage: &str,
    ) -> Option<String> {
        if self.args.notice_envelope
            && let Some(input) = pasted
                .and_then(|text| send_submit::known_multiline_input(&self.record.agent, pane, text))
        {
            return Some(input);
        }
        if let Some(input) = send_submit::exact_composer_input(&self.record.agent, pane) {
            return Some(input);
        }
        // Only a send admitted against an empty composer owns the known paste.
        // Cleanup's general joined-row fallback is not submit evidence.
        if !self.args.require_empty_composer {
            return None;
        }
        let text = pasted?;
        let agent = self.record.agent.as_str();
        let (input, end) = send_submit::wrapped_composer_layout(agent, pane, text)?;
        if input != text {
            // The wrapped composer is readable, but does not contain only our
            // paste. Preserve the mismatch as an immediate refusal instead of
            // treating it as a transient observation and retrying into a later
            // composer state that may no longer show the owner draft.
            return Some(input);
        }
        let target = format!("{}:0.0", self.record.tmux_session);
        let steps = text.chars().count();
        let last_cells = text.chars().next_back()?.width()?;
        // A visual layout may omit or pad spaces. Check the logical length:
        // N-1 characters must stop one character before the known endpoint,
        // and one more must reach it. This rejects both extra and missing separators.
        // A matched final row contains a nonempty paste remainder, so its
        // endpoint is past column 2 and the preceding cell is defined.
        let mut navigation = vec![
            (&["C-a"][..], 1, None),
            (
                &["Right"][..],
                steps.checked_sub(1)?,
                Some((end.0 - last_cells, end.1)),
            ),
            (&["Right"][..], 1, Some(end)),
        ];
        if agent == "codex" {
            // A matched prefix can hide an owner-appended blank. Right then
            // Left must return to N-1 only when N is the actual logical end.
            // Observing that changed position also avoids accepting a stale
            // unchanged cursor before the extra Right has been processed.
            navigation.extend([
                (&["Right", "Left"][..], 1, Some((end.0 - last_cells, end.1))),
                (&["Right"][..], 1, Some(end)),
            ]);
        }
        for (keys, count, endpoint) in navigation {
            self.identity(stage).ok()?;
            let mut command = Command::new(self.tmux_bin);
            command.args(["send-keys", "-t", &target, "-N", &count.to_string()]);
            command.args(keys);
            run_status_with_timeout(command, "tmux send-keys", self.timeout(stage).ok()?).ok()?;
            let until = Instant::now() + crate::POST_PASTE_KEY_SETTLE_DELAY;
            loop {
                let (current, cursor) = observation::pane_and_cursor(
                    self.tmux_bin,
                    &target,
                    self.timeout(stage).ok()?,
                )?;
                match classify_wrapped_proof(
                    &input,
                    send_submit::wrapped_composer_layout(agent, &current, text)
                        .map(|(input, _)| input),
                ) {
                    WrappedProofObservation::Exact(_) => {}
                    WrappedProofObservation::Mismatch(input) => return Some(input),
                    WrappedProofObservation::Unobservable => return None,
                }
                let reached = endpoint.map_or_else(
                    || send_submit::cleanup_cursor_at_start(agent, &current, cursor.0, cursor.1),
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

#[cfg(test)]
mod tests {
    use super::{WrappedProofObservation, classify_wrapped_proof};

    #[test]
    fn readable_wrapped_mismatch_is_not_an_unobservable_retry() {
        assert_eq!(
            classify_wrapped_proof("owned paste", Some("owned paste user draft".to_string())),
            WrappedProofObservation::Mismatch("owned paste user draft".to_string())
        );
    }
}
