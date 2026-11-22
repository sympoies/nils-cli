//! Claude Code 2.1.330 renders background-work counts below its composer
//! and provides an Escape action on both supported exit dialogs.

use super::{CliContext, CliError, Instant, Path, SessionRecord, json};
use crate::cli::SpecialKey;
use crate::send_guard::capture_styled;
use crate::send_submit::{claude_composer, exact_composer_input};
use nils_common::shell::{AnsiStripMode, strip_ansi};
use std::time::Duration;

const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(1);

fn remaining(deadline: Instant) -> Duration {
    OBSERVATION_TIMEOUT.min(deadline.saturating_duration_since(Instant::now()))
}

pub(super) fn require_no_background_work(
    tmux: &Path,
    record: &SessionRecord,
    deadline: Instant,
) -> Result<(), CliError> {
    let timeout = remaining(deadline);
    let target = crate::managed_tmux_pane_target(&record.tmux_session);
    let pane = (!timeout.is_zero())
        .then(|| capture_styled(tmux, &target, timeout))
        .flatten();
    let (cause, message) = match pane {
        Some(pane) if has_background_work(&pane) => (
            Dialog::BackgroundWork.code(),
            "not restarted: Claude has running background work",
        ),
        Some(pane) if exact_composer_input("claude", &pane).as_deref() == Some("") => return Ok(()),
        _ => (
            "send-composer-unobservable",
            "not restarted: Claude's empty composer and background-work footer could not be verified",
        ),
    };
    Err(CliError::runtime(
        "restart-session-not-idle",
        message,
        Some(json!({"id": record.id, "cause": cause})),
    ))
}

fn has_background_work(pane: &str) -> bool {
    let plain = strip_ansi(pane, AnsiStripMode::CsiSgrOnly);
    let lines: Vec<_> = plain.lines().collect();
    claude_composer(&lines).is_some_and(|(_, footer)| {
        footer
            .iter()
            .flat_map(|line| line.split(['·', ',']))
            .any(|item| {
                let item = item.trim();
                if let Some(count) = item
                    .strip_prefix("↳ ")
                    .and_then(|item| item.strip_suffix(" background"))
                {
                    return count.parse::<u64>().is_ok_and(|count| count > 0);
                }
                let Some((count, kind)) = item.split_once(' ') else {
                    return false;
                };
                count.parse::<u64>().is_ok_and(|count| count > 0)
                    && matches!(
                        kind,
                        "shell"
                            | "shells"
                            | "monitor"
                            | "monitors"
                            | "background task"
                            | "background tasks"
                            | "background agent"
                            | "background agents"
                            | "background dynamic workflow"
                            | "background dynamic workflows"
                            | "scheduled task"
                            | "scheduled tasks"
                            | "MCP task"
                            | "MCP tasks"
                            | "Artifact comment monitor"
                            | "Artifact comment monitors"
                    )
            })
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dialog {
    BackgroundWork,
    FeedbackDraft,
}

impl Dialog {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::BackgroundWork => "claude-background-work",
            Self::FeedbackDraft => "claude-feedback-draft",
        }
    }

    pub(super) fn unresolved_message(self) -> &'static str {
        match self {
            Self::BackgroundWork => {
                "not restarted: could not verify cancellation of Claude's background-work exit dialog"
            }
            Self::FeedbackDraft => {
                "not restarted: Claude did not exit after showing the unsent feedback draft prompt"
            }
        }
    }

    fn detect(pane: &str) -> Option<Self> {
        let plain = strip_ansi(pane, AnsiStripMode::CsiSgrOnly);
        let mut rows = plain
            .lines()
            .map(str::trim)
            .filter(|row| !row.is_empty())
            .rev();
        let last = rows.next()?;
        // Match the active prompt at the end of the visible pane, including
        // the actions, so earlier transcript text cannot authorize a key.
        let mut tail: Vec<_> = std::iter::once(last).chain(rows.clone().take(4)).collect();
        tail.reverse();
        let feedback_tail = tail.join(" ").to_ascii_lowercase();
        if let Some(before_actions) =
            feedback_tail.strip_suffix(" enter to review & send · esc to discard and exit")
            && let Some((_, draft)) = before_actions.rsplit_once("you have ")
            && let Some((count, label)) = draft.split_once(' ')
            && count.parse::<u64>().is_ok_and(|count| count > 0)
            && matches!(label, "unsent feedback draft" | "unsent feedback drafts")
        {
            return Some(Self::FeedbackDraft);
        }

        let stay = if last.to_ascii_lowercase().ends_with("esc to cancel") {
            rows.next()?
        } else {
            last
        };
        if option_label(stay) != "Stay" {
            return None;
        }
        let mut exit = rows.next()?;
        if option_label(exit) == "Move to background and exit" {
            exit = rows.next()?;
        }
        (option_label(exit) == "Exit and stop tasks"
            && rows.any(|row| row == "Background work is running"))
        .then_some(Self::BackgroundWork)
    }
}

fn option_label(row: &str) -> &str {
    let row = row.trim_start_matches(['❯', '›']).trim_start();
    match row.split_once(". ") {
        Some((number, label)) if number.parse::<u8>().is_ok() => label,
        _ => row,
    }
}

#[derive(Default)]
pub(super) struct ExitDialogs {
    seen: Option<Dialog>,
    attempted: Option<Dialog>,
}

impl ExitDialogs {
    pub(super) fn last_dialog(&self) -> Option<Dialog> {
        self.seen
    }

    /// Returns true once the original pane again shows its empty composer
    /// after Escape selected Stay. A feedback Escape instead waits for exit.
    pub(super) fn poll(
        &mut self,
        context: &CliContext,
        expected: &SessionRecord,
        tmux: &Path,
        deadline: Instant,
    ) -> bool {
        self.poll_inner(context, expected, tmux, deadline)
            .unwrap_or(false)
    }

    fn poll_inner(
        &mut self,
        context: &CliContext,
        expected: &SessionRecord,
        tmux: &Path,
        deadline: Instant,
    ) -> Option<bool> {
        let identity = crate::persisted_tmux_runtime_identity(expected).ok()??;
        let pane = capture_styled(tmux, &identity.pane_id, remaining(deadline))?;
        let dialog = Dialog::detect(&pane);
        self.seen = dialog.or(self.seen);
        let verify_rollback = self.attempted == Some(Dialog::BackgroundWork) && dialog.is_none();
        if !verify_rollback
            && (dialog.is_none()
                || dialog == self.attempted
                || self.attempted == Some(Dialog::BackgroundWork))
        {
            return None;
        }

        // Serialize against lifecycle writers and re-observe the pinned pane
        // immediately before any input. The restart fence still excludes sends.
        let _lock = crate::try_acquire_session_record_lock(context, &expected.id).ok()??;
        let current = crate::load_session_record(context, &expected.id).ok()?;
        crate::ensure_same_session_identity(expected, &current).ok()?;
        if !crate::tmux_pane_identity_matches(
            tmux,
            &current,
            &identity.session_id,
            &identity.pane_id,
            identity.pane_pid,
            remaining(deadline),
        ) {
            return None;
        }
        let pane = capture_styled(tmux, &identity.pane_id, remaining(deadline))?;
        if verify_rollback {
            return Some(exact_composer_input("claude", &pane).as_deref() == Some(""));
        }
        let timeout = remaining(deadline);
        if Dialog::detect(&pane) == dialog && !timeout.is_zero() {
            // A timeout may still have delivered the key. Use at most one
            // Escape per dialog, then require the expected pane transition.
            self.attempted = dialog;
            let _ = crate::send_tmux_key_with_timeout(
                tmux,
                &identity.pane_id,
                SpecialKey::Escape,
                timeout,
            );
        }
        None
    }
}
