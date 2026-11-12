//! Ownership is local to one paste after an observed empty admission.
use super::{Admission, GuardArgs, capture_styled, observation};
use crate::{CliContext, CliError, SessionRecord, send_submit};
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

impl Admission<'_> {
    pub(crate) fn owned_placeholder(&self) -> Option<String> {
        if self.record.agent != "claude" || !self.args.require_empty_composer {
            return None;
        }
        let target = format!("{}:0.0", self.record.tmux_session);
        let pane = capture_styled(self.tmux_bin, &target, self.timeout("submit").ok()?)?;
        send_submit::claude_paste_placeholder(&pane)
    }

    pub(super) fn prove_placeholder(&self, label: &str) -> bool {
        Navigation {
            context: self.context,
            record: self.record,
            tmux: self.tmux_bin,
            deadline: self.deadline,
        }
        .prove(label)
    }

    pub(crate) fn bind_placeholder(&self, known: &mut Option<String>) -> Result<(), CliError> {
        if let Some(label) = self.owned_placeholder() {
            if known.as_ref().is_some_and(|before| before != &label) {
                return Err(self.refusal(
                    "send-composer-mismatch",
                    "paste placeholder changed",
                    "composer",
                    "submit",
                ));
            }
            *known = Some(label);
        }
        Ok(())
    }

    pub(crate) fn report_owned_input(
        &self,
        error: CliError,
        text: &str,
        observed: bool,
        label: Option<&str>,
    ) -> CliError {
        let mut result = self.clear_pasted(text, observed);
        if self.record.agent == "claude"
            && self.args.require_empty_composer
            && (label.is_some()
                || result
                    .0
                    .as_deref()
                    .is_none_or(|input| input.starts_with("[Pasted text #")))
        {
            // A complete atomic-token proof needs four pairs of stable cursor
            // observations, each with an identity read, plus guarded navigation.
            // Use the terminal budget even when the submit budget has expired.
            let deadline = Instant::now() + crate::send::guarded_hold_budget(self.record, false);
            let label = label
                .map(str::to_owned)
                .or_else(|| observe(self.record, self.tmux_bin, deadline));
            if let Some(label) = label {
                let navigation = Navigation {
                    context: self.context,
                    record: self.record,
                    tmux: self.tmux_bin,
                    deadline,
                };
                if navigation.clear(&label) {
                    result = (Some(String::new()), true);
                }
            }
        }
        self.report_cleared_input(error, text, observed, result)
    }
}

/// Observe throughout one empty-admitted send; normal Console delivery still
/// receives its authoritative provider acknowledgement after the lock releases.
fn submit(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    label: Option<&str>,
    text: &str,
    deadline: Instant,
    mut enter: impl FnMut() -> Result<(), CliError>,
) -> Result<send_submit::SubmitReport, CliError> {
    let navigation = Navigation {
        context,
        record,
        tmux,
        deadline,
    };
    let target = format!("{}:0.0", record.tmux_session);
    let represented = label.unwrap_or(text);
    // Whitespace has no text probe. Only an actual opaque marker can establish
    // pending input; ordinary whitespace keeps its single-Enter receipt.
    let probe_text = if represented.chars().all(char::is_whitespace) {
        "[Pasted text #1]"
    } else {
        represented
    };
    let probe = send_submit::probe(probe_text).expect("nonempty text or paste marker probe");
    let known = RefCell::new(label.map(str::to_owned));
    let changed = Cell::new(false);
    let result = send_submit::submit_and_confirm(
        "claude",
        &probe,
        deadline,
        |timeout| {
            let pane = crate::capture_visible_pane_with_timeout(tmux, &target, timeout);
            if let Some(current) = pane
                .as_deref()
                .and_then(send_submit::claude_paste_placeholder)
            {
                if known
                    .borrow()
                    .as_ref()
                    .is_some_and(|label| label != &current)
                {
                    changed.set(true);
                } else {
                    *known.borrow_mut() = Some(current);
                }
            }
            pane
        },
        |timeout| crate::send::recorded_tmux_runtime_stopped(tmux, record, timeout),
        || {
            !changed.get()
                && crate::activity::state_for_view(context, record)
                    .is_none_or(|turn| turn.phase != crate::activity::TurnPhase::NeedsInput)
        },
        || {
            let pane = capture_styled(tmux, &target, navigation.remaining()).ok_or_else(|| {
                CliError::runtime(
                    "send-composer-unobservable",
                    "paste composer is unobservable",
                    None,
                )
            })?;
            if let Some(current) = send_submit::claude_paste_placeholder(&pane) {
                if !navigation.identity() {
                    return Err(CliError::runtime(
                        "send-identity-mismatch",
                        "paste runtime identity changed",
                        None,
                    ));
                }
                if changed.get()
                    || known
                        .borrow()
                        .as_ref()
                        .is_some_and(|label| label != &current)
                {
                    return Err(CliError::runtime(
                        "send-composer-mismatch",
                        "paste placeholder changed",
                        None,
                    ));
                }
                *known.borrow_mut() = Some(current.clone());
                if !navigation.prove(&current) {
                    return Err(CliError::runtime(
                        "send-composer-mismatch",
                        "paste placeholder is not the complete logical input",
                        None,
                    ));
                }
            } else if known.borrow().is_some() || send_submit::has_claude_paste_placeholder(&pane) {
                return Err(CliError::runtime(
                    "send-composer-mismatch",
                    "paste placeholder changed or has adjacent input",
                    None,
                ));
            } else if let Ok(input) = send_submit::direct_composer_observation("claude", &pane)
                && !input.is_empty()
                && !send_submit::composer_holds("claude", &input, text)
            {
                return Err(CliError::runtime(
                    "send-composer-mismatch",
                    "composer contains different input",
                    None,
                ));
            }
            enter()
        },
    );
    let mut known = known.into_inner();
    let failure = match result {
        Ok(report)
            if !changed.get()
                && matches!(
                    report.outcome,
                    send_submit::SubmitOutcome::Submitted | send_submit::SubmitOutcome::Queued
                ) =>
        {
            return Ok(report);
        }
        Ok(report) => {
            // Nonopaque sends retain their receipt contract. The HTTP owner
            // separately requires provider acknowledgement after lock release.
            if known.is_none() {
                known = observe(
                    record,
                    tmux,
                    Instant::now() + crate::PANE_OBSERVATION_COMMAND_TIMEOUT,
                );
                if known.is_none() {
                    return Ok(report);
                }
            }
            CliError::runtime(
                "send-submit-unconfirmed",
                "prompt submission was not confirmed",
                Some(
                    json!({"id":record.id,"outcome":report.outcome,"enter_presses":report.enter_presses,"atomic":false}),
                ),
            )
        }
        Err(error) => error,
    };
    let known =
        known.or_else(|| observe(record, tmux, Instant::now() + Duration::from_millis(2500)));
    if let Some(label) = known {
        return Err(report_failure(
            context,
            record,
            tmux,
            text,
            failure,
            Some(&label),
        ));
    }
    let mut error = failure;
    let details = error.0.details.get_or_insert_with(|| json!({}));
    details["pasted"] = json!(true);
    details["cleared"] = json!(false);
    details["remaining_input"] = json!("unobservable");
    Err(error)
}

pub(crate) fn submit_after_empty(
    context: &CliContext,
    record: &SessionRecord,
    text: &str,
    tmux: &Path,
    deadline: Instant,
    enter: impl FnMut() -> Result<(), CliError>,
) -> Result<send_submit::SubmitReport, CliError> {
    submit(context, record, tmux, None, text, deadline, enter)
}

struct Navigation<'a> {
    context: &'a CliContext,
    record: &'a SessionRecord,
    tmux: &'a Path,
    deadline: Instant,
}
impl Navigation<'_> {
    fn remaining(&self) -> Duration {
        self.deadline
            .saturating_duration_since(Instant::now())
            .min(crate::PANE_OBSERVATION_COMMAND_TIMEOUT)
    }
    fn identity(&self) -> bool {
        if self.remaining().is_zero() {
            return false;
        }
        let args = GuardArgs::default();
        Admission {
            context: self.context,
            record: self.record,
            args: &args,
            tmux_bin: self.tmux,
            verify_composer: false,
            deadline: self.deadline,
        }
        .identity("cleanup")
        .is_ok()
    }
    fn key(&self, keys: &[&str], clear: bool) -> bool {
        if !self.identity() {
            return false;
        }
        let target = format!("{}:0.0", self.record.tmux_session);
        let mut cmd = Command::new(self.tmux);
        cmd.args(["send-keys", "-t", &target, "-N", "1"]).args(keys);
        let sent = || crate::run_status_with_timeout(cmd, "tmux send-keys", self.remaining());
        if clear {
            return super::response::watch(
                self.context,
                self.record,
                self.tmux,
                super::response::Operation::Clear,
                sent,
            )
            .is_ok();
        }
        sent().is_ok()
    }
    fn cursor(&self, label: &str, expected: Option<(usize, usize)>) -> Option<(usize, usize)> {
        let target = format!("{}:0.0", self.record.tmux_session);
        // Allow a stale first read and two complete, identity-fenced cursor
        // observations. Subprocess time counts as well as renderer settling.
        let until = (Instant::now() + Duration::from_secs(2)).min(self.deadline);
        let mut stable = 0;
        loop {
            if !self.identity() {
                return None;
            }
            let (pane, cursor) =
                observation::pane_and_cursor(self.tmux, &target, self.remaining())?;
            if send_submit::claude_paste_placeholder(&pane).as_deref() != Some(label) {
                return None;
            }
            if expected.map_or_else(
                || send_submit::cleanup_cursor_at_start("claude", &pane, cursor.0, cursor.1),
                |wanted| wanted == cursor,
            ) {
                stable += 1;
                if stable == 2 {
                    return Some(cursor);
                }
            } else {
                stable = 0;
            }
            if Instant::now() >= until {
                return None;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    fn prove(&self, label: &str) -> bool {
        let target = format!("{}:0.0", self.record.tmux_session);
        if capture_styled(self.tmux, &target, self.remaining())
            .and_then(|pane| send_submit::claude_paste_placeholder(&pane))
            .as_deref()
            != Some(label)
        {
            return false;
        }
        if !self.key(&["C-a"], false) {
            return false;
        }
        let Some(start) = self.cursor(label, None) else {
            return false;
        };
        let end = (start.0 + label.len(), start.1);
        // Observe one atomic paste block and prove the logical end. A hidden
        // owner suffix makes Right/Left return to end, rather than start.
        for (keys, position) in [
            (&["Right"][..], end),
            (&["Right", "Left"][..], start),
            (&["Right"][..], end),
        ] {
            if !self.key(keys, false) || self.cursor(label, Some(position)).is_none() {
                return false;
            }
        }
        true
    }
    fn clear(&self, label: &str) -> bool {
        if !self.prove(label) {
            return false;
        }
        if !self.key(&["BSpace"], true) {
            return false;
        }
        let target = format!("{}:0.0", self.record.tmux_session);
        while !self.remaining().is_zero() {
            if !self.identity() {
                return false;
            }
            if let Some(pane) = capture_styled(self.tmux, &target, self.remaining()) {
                if send_submit::exact_composer_input("claude", &pane).as_deref() == Some("") {
                    return true;
                }
                if send_submit::claude_paste_placeholder(&pane).as_deref() != Some(label) {
                    return false;
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

pub(crate) fn observe(record: &SessionRecord, tmux: &Path, deadline: Instant) -> Option<String> {
    if record.agent != "claude" {
        return None;
    }
    let target = format!("{}:0.0", record.tmux_session);
    capture_styled(
        tmux,
        &target,
        deadline
            .saturating_duration_since(Instant::now())
            .min(crate::PANE_OBSERVATION_COMMAND_TIMEOUT),
    )
    .and_then(|pane| send_submit::claude_paste_placeholder(&pane))
}

/// Plain and partial-guard submissions must establish the same empty boundary.
/// This observation never retires or clears an existing owner draft.
pub(crate) fn admit(record: &SessionRecord, tmux: &Path) -> Result<(), CliError> {
    let target = format!("{}:0.0", record.tmux_session);
    let input = capture_styled(tmux, &target, crate::PANE_OBSERVATION_COMMAND_TIMEOUT)
        .ok_or(send_submit::Unobserved::CaptureFailed)
        .and_then(|pane| send_submit::direct_composer_observation("claude", &pane));
    if input.as_deref() == Ok("") {
        return Ok(());
    }
    let (code, message) = if input.is_ok() {
        (
            "send-composer-not-empty",
            "composer contains an owner draft",
        )
    } else {
        (
            "send-composer-unobservable",
            "composer content cannot be observed exactly",
        )
    };
    let mut details = json!({"id":record.id,"stage":"admission","pasted":false,"submitted":false,"atomic":false,"guard":"empty_composer"});
    if let Err(reason) = input {
        details["observation"] = json!(reason.as_str());
    }
    Err(CliError::runtime(code, message, Some(details)))
}

pub(crate) fn report_failure(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    text: &str,
    error: CliError,
    label: Option<&str>,
) -> CliError {
    let args = GuardArgs {
        require_empty_composer: true,
        ..Default::default()
    };
    let admission = Admission {
        context,
        record,
        args: &args,
        tmux_bin: tmux,
        verify_composer: true,
        deadline: Instant::now() + Duration::from_millis(2500),
    };
    admission.report_owned_input(error, text, true, label)
}
