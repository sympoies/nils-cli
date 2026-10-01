//! Verified submission for `send --text … --key enter`.
//!
//! tmux only confirms that it wrote bytes to the pane. Claude Code and Codex can
//! still drop the Enter that follows a paste and leave the prompt sitting in
//! their composer, where nobody notices it for hours. A text-and-Enter send
//! therefore reads the pane back, presses Enter again a bounded number of times
//! while its own text is still visibly in the composer, and reports what it
//! saw. Every decision here is about the composer region only: anything the
//! classifier does not recognize as a composer (a dialog, an unknown provider,
//! an unreadable pane) is never answered with another Enter.

use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Characters of the first non-blank line used to find the text again. Short
/// enough to survive the provider wrapping a long line, long enough not to match
/// a placeholder by accident.
const PROBE_CHARS: usize = 20;
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// How long the text may stay in the composer after an Enter before that Enter
/// counts as swallowed.
const RETRY_AFTER: Duration = Duration::from_millis(1500);
/// The caller's Enter plus one retry for the swallowed first Enter.
const MAX_ENTER_PRESSES: u32 = 2;
/// Upper bound on the whole confirmation. `send` holds the session record lock
/// throughout, and a provider hook event that finds the lock busy is retried
/// for only five seconds, so the loop must end well before that.
const CONFIRM_DEADLINE: Duration = Duration::from_millis(3500);
/// Consecutive unrecognized polls after the text was seen before the loop stops
/// waiting: a dialog has replaced the composer, and nothing will be pressed.
const UNRECOGNIZED_POLLS_AFTER_PENDING: u32 = 2;

/// Placeholders the providers show instead of a long paste.
const PASTE_PLACEHOLDERS: [&str; 2] = ["[Pastedtext#", "[PastedContent"];
const CLAUDE_QUEUED_HINT: &str = "Pressuptoeditqueuedmessages";
/// Footer phrases of provider dialogs. Their presence below the composer means
/// an Enter would answer the dialog instead of submitting the prompt.
const DIALOG_HINTS: [&str; 5] = [
    "enter to confirm",
    "enter select",
    "esc to cancel",
    "esc back",
    "do you want to proceed",
];
/// Codex draws at most a short status footer below its composer.
const CODEX_MAX_FOOTER_LINES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Composer {
    /// The composer is visible and still holds the sent text.
    Pending,
    /// The composer is visible and no longer holds the sent text.
    Clear,
    /// Claude Code accepted the text into its queue behind a running turn.
    Queued,
    /// No composer could be identified; nothing may be pressed.
    Unrecognized,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubmitOutcome {
    /// The text left the composer after an Enter.
    Submitted,
    /// The provider queued the text behind its running turn.
    Queued,
    /// The text is still in the composer after every allowed Enter.
    Stuck,
    /// The pane could not prove either way (unknown provider or layout, a
    /// dialog, or an unreadable pane).
    Unverified,
}

impl SubmitOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Queued => "queued",
            Self::Stuck => "stuck",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SubmitReport {
    pub(crate) outcome: SubmitOutcome,
    pub(crate) enter_presses: u32,
}

/// The whitespace-free start of the text's first non-blank line, or `None`
/// when the text has nothing to find again.
pub(crate) fn probe(text: &str) -> Option<String> {
    let line = text.lines().find(|line| !line.trim().is_empty())?;
    Some(compact(line).chars().take(PROBE_CHARS).collect())
}

/// Whitespace is dropped before matching so a wrapped line and its
/// continuation indent still match the probe.
fn compact(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

pub(crate) fn classify(agent: &str, pane: &str, probe: &str) -> Composer {
    let lines: Vec<&str> = pane.lines().map(str::trim_end).collect();
    let located = match agent {
        "claude" => claude_composer(&lines),
        "codex" => codex_composer(&lines),
        _ => None,
    };
    let Some((region, footer)) = located else {
        return Composer::Unrecognized;
    };
    let shows_dialog = footer.iter().any(|line| {
        let line = line.to_lowercase();
        DIALOG_HINTS.iter().any(|hint| line.contains(hint))
    });
    if shows_dialog {
        return Composer::Unrecognized;
    }
    let region = compact(&region.join("\n"));
    if region.contains(probe) || PASTE_PLACEHOLDERS.iter().any(|mark| region.contains(mark)) {
        Composer::Pending
    } else if agent == "claude" && region.contains(CLAUDE_QUEUED_HINT) {
        Composer::Queued
    } else {
        Composer::Clear
    }
}

/// Claude Code draws its input box between the last two horizontal rules and
/// starts it with `❯`. A permission or trust dialog replaces the box, so no
/// such region exists while one is open.
fn claude_composer<'a>(lines: &'a [&'a str]) -> Option<(&'a [&'a str], &'a [&'a str])> {
    let rules: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.trim_start().starts_with('─') && line.chars().filter(|ch| *ch == '─').count() >= 10
        })
        .map(|(index, _)| index)
        .collect();
    let [.., top, bottom] = rules[..] else {
        return None;
    };
    let region = &lines[top + 1..bottom];
    let first = region.first()?.trim_start();
    let input = first.strip_prefix('❯')?;
    if starts_with_numbered_choice(input) {
        return None;
    }
    Some((region, &lines[bottom + 1..]))
}

/// Codex draws its composer as the last `›` line plus indented continuation
/// lines, followed only by a short status footer. Its transcript echoes earlier
/// prompts with the same `›`, so a `›` block followed by more than a footer is
/// transcript, not the composer.
fn codex_composer<'a>(lines: &'a [&'a str]) -> Option<(&'a [&'a str], &'a [&'a str])> {
    let start = lines.iter().rposition(|line| line.starts_with('›'))?;
    if starts_with_numbered_choice(&lines[start]['›'.len_utf8()..]) {
        return None;
    }
    let end = lines[start + 1..]
        .iter()
        .position(|line| !line.starts_with("  ") || line.trim().is_empty())
        .map_or(lines.len(), |offset| start + 1 + offset);
    let footer = &lines[end..];
    let footer_lines = footer.iter().filter(|line| !line.trim().is_empty()).count();
    (footer_lines <= CODEX_MAX_FOOTER_LINES).then_some((&lines[start..end], footer))
}

/// Provider selection lists draw their cursor as the composer glyph followed by
/// `1.`, `2.` …; such a line is a choice, not typed input.
fn starts_with_numbered_choice(after_glyph: &str) -> bool {
    let rest = after_glyph.trim_start_matches([' ', '\u{a0}']);
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with('.')
}

/// Press the submitting Enter, then watch the composer until it proves the
/// outcome. `observe` returns the visible pane (`None` when tmux cannot answer),
/// `may_press` is consulted before every retry so a session recorded as waiting
/// on a dialog is never answered, and `press_enter` sends one Enter. Provider
/// events cannot land while the caller holds the session lock, so the pane
/// classifier, which never sees a composer behind a dialog, is the primary
/// guard and `may_press` covers state recorded before the send.
pub(crate) fn submit_and_confirm(
    agent: &str,
    probe: &str,
    mut observe: impl FnMut() -> Option<String>,
    mut may_press: impl FnMut() -> bool,
    mut press_enter: impl FnMut() -> Result<(), crate::CliError>,
) -> Result<SubmitReport, crate::CliError> {
    let view = |pane: Option<String>| {
        pane.map_or(Composer::Unrecognized, |pane| classify(agent, &pane, probe))
    };
    // A blank capture is a pane tmux could not show us, not an empty composer.
    let before = observe().filter(|pane| !pane.trim().is_empty());
    let observable = before.is_some();
    let mut seen_pending = view(before) == Composer::Pending;
    press_enter()?;
    let mut presses = 1;
    let started = Instant::now();
    let mut last_press = started;
    let report = |outcome, presses| SubmitReport {
        outcome,
        enter_presses: presses,
    };
    if !observable {
        return Ok(report(SubmitOutcome::Unverified, presses));
    }
    let mut unrecognized_polls = 0;
    loop {
        thread::sleep(POLL_INTERVAL);
        let current = view(observe());
        unrecognized_polls = if current == Composer::Unrecognized {
            unrecognized_polls + 1
        } else {
            0
        };
        match current {
            Composer::Queued => return Ok(report(SubmitOutcome::Queued, presses)),
            Composer::Clear if seen_pending => {
                return Ok(report(SubmitOutcome::Submitted, presses));
            }
            Composer::Pending => {
                seen_pending = true;
                if last_press.elapsed() >= RETRY_AFTER {
                    if presses >= MAX_ENTER_PRESSES || !may_press() {
                        return Ok(report(SubmitOutcome::Stuck, presses));
                    }
                    press_enter()?;
                    presses += 1;
                    last_press = Instant::now();
                }
            }
            // The text was never seen in a composer, so there is nothing to
            // retry and no proof to wait for.
            Composer::Clear | Composer::Unrecognized
                if !seen_pending && started.elapsed() >= RETRY_AFTER =>
            {
                return Ok(report(SubmitOutcome::Unverified, presses));
            }
            Composer::Unrecognized if unrecognized_polls >= UNRECOGNIZED_POLLS_AFTER_PENDING => {
                return Ok(report(SubmitOutcome::Unverified, presses));
            }
            Composer::Clear | Composer::Unrecognized => {}
        }
        if started.elapsed() >= CONFIRM_DEADLINE {
            let outcome = if current == Composer::Pending {
                SubmitOutcome::Stuck
            } else {
                SubmitOutcome::Unverified
            };
            return Ok(report(outcome, presses));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;

    // Pane captures from Claude Code 2.1.284 and Codex 0.159.0 (2026-10-01).
    const CLAUDE_PENDING: &str = "❯ This is a scratch test session. Reply only with OK.

● OK

✻ Sautéed for 2s · done 9:06 AM

────────────────────────────── scratch multiline submit repro (sympoies-infra) ─
❯\u{a0}[Monitoring] REPRO A1: Disk usage high (scratch test, reply OK)
  /data is 91% full
  threshold 90%
────────────────────────────────────────────────────────────────────────────────
  [Opus 5.5 ◑ high] │ scratch-cwd
  ⏵⏵ bypass permissions on (shift+tab to cycle)
";
    const CLAUDE_SUBMITTED: &str =
        "❯ [Monitoring] REPRO A1: Disk usage high (scratch test, reply OK)
  /data is 91% full
  threshold 90%

● OK

────────────────────────────── scratch multiline submit repro (sympoies-infra) ─
❯\u{a0}
────────────────────────────────────────────────────────────────────────────────
  [Opus 5.5 ◑ high] │ scratch-cwd
";
    const CLAUDE_PASTED_PLACEHOLDER: &str = "────────────────────────────────────────
❯\u{a0}[Pasted text #2 +12 lines]
────────────────────────────────────────
  paste again to expand
";
    const CLAUDE_STARTUP_PLACEHOLDER: &str = "────────────────────────────────────────
❯\u{a0}Try \"edit <filepath> to...\"
────────────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents
";
    const CLAUDE_QUEUED: &str = "  Second queued message.
  ctrl+x ctrl+s to send now
· Galloping… (running PreToolUse hook · 4s)
────────────────────────────────────────
❯\u{a0}Press up to edit queued messages
────────────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents
";
    const CLAUDE_TRUST_DIALOG: &str = "────────────────────────────────────────
 Accessing workspace:

 Quick safety check: Is this a project you created or one you trust?

 ❯ No, exit
   Yes, I trust this folder

 Enter to confirm · Esc to cancel
";
    const CLAUDE_CHOICE_BETWEEN_RULES: &str = "────────────────────────────────────────
❯ 1. Yes
  2. No
────────────────────────────────────────
";
    const CODEX_PENDING: &str = "  >_ OpenAI Codex (v0.159.0)
  permissions: YOLO mode

› Scratch test: reply only OK.
  line two
  line three

  GPT-6.1-Sol high · Context 100% left · /tmp/probe
";
    const CODEX_SUBMITTED: &str = "› Scratch test: reply only OK.
  line two
  line three

• Working (0s • esc to interrupt)

› Ask Codex to do anything

  GPT-6.1-Sol high · Context 100% left · /tmp/probe
  ← for agents · ? for shortcuts
";
    const CODEX_DIALOG: &str = "› Scratch test: reply only OK.
  line two
  line three

■ Your workspace is out of credits.
  Usage limit reached
  Request a limit increase from your owner to continue using codex. Request increase?
  1. Yes (y)
› 2. No (default) (n)

  enter select · esc back
";
    const CODEX_ECHO_ABOVE_APPROVAL: &str = "› Scratch test: reply only OK.

  Would you like to run the following command?
  $ rm -rf build
  Reason: clean the build directory
  Yes, proceed
  No, and tell Codex what to do differently
  Press enter to confirm or esc to cancel
";

    fn probe_of(text: &str) -> String {
        probe(text).expect("probe")
    }

    #[test]
    fn claude_composer_states_follow_real_panes() {
        let probe = probe_of("[Monitoring] REPRO A1: Disk usage high\n/data is 91% full");
        assert_eq!(
            classify("claude", CLAUDE_PENDING, &probe),
            Composer::Pending
        );
        assert_eq!(
            classify("claude", CLAUDE_SUBMITTED, &probe),
            Composer::Clear
        );
        assert_eq!(
            classify("claude", CLAUDE_PASTED_PLACEHOLDER, &probe),
            Composer::Pending
        );
        assert_eq!(
            classify("claude", CLAUDE_STARTUP_PLACEHOLDER, &probe),
            Composer::Clear
        );
        assert_eq!(classify("claude", CLAUDE_QUEUED, &probe), Composer::Queued);
    }

    #[test]
    fn claude_dialogs_are_never_a_composer() {
        let probe = probe_of("No, exit");
        assert_eq!(
            classify("claude", CLAUDE_TRUST_DIALOG, &probe),
            Composer::Unrecognized
        );
        assert_eq!(
            classify("claude", CLAUDE_CHOICE_BETWEEN_RULES, &probe_of("1. Yes")),
            Composer::Unrecognized
        );
    }

    #[test]
    fn codex_composer_states_follow_real_panes() {
        let probe = probe_of("Scratch test: reply only OK.\nline two\nline three");
        assert_eq!(classify("codex", CODEX_PENDING, &probe), Composer::Pending);
        assert_eq!(classify("codex", CODEX_SUBMITTED, &probe), Composer::Clear);
        assert_eq!(
            classify("codex", CODEX_DIALOG, &probe),
            Composer::Unrecognized
        );
        assert_eq!(
            classify("codex", CODEX_ECHO_ABOVE_APPROVAL, &probe),
            Composer::Unrecognized,
            "a transcript echo above a dialog must not read as a pending composer"
        );
    }

    #[test]
    fn unknown_providers_and_blank_text_are_unverifiable() {
        assert_eq!(
            classify("hermes", CLAUDE_PENDING, &probe_of("[Monitoring]")),
            Composer::Unrecognized
        );
        assert_eq!(probe(" \n\t\n"), None);
        assert_eq!(
            probe_of("  first line that is long enough to cut\nsecond"),
            "firstlinethatislongenoughtocut"[..20].to_string()
        );
    }

    #[test]
    fn a_wrapped_long_line_still_matches_its_probe() {
        let pane = "────────────────────\n❯\u{a0}Scratch test, reply\n  only OK. filler\n────────────────────\n";
        assert_eq!(
            classify(
                "claude",
                pane,
                &probe_of("Scratch test, reply only OK. filler")
            ),
            Composer::Pending
        );
    }

    fn scripted(
        panes: &[&str],
        may_press: bool,
    ) -> (Result<SubmitReport, crate::CliError>, u32, usize) {
        let probe = probe_of("[Monitoring] REPRO A1: Disk usage high");
        let observed = RefCell::new(0usize);
        let pressed = RefCell::new(0u32);
        let result = submit_and_confirm(
            "claude",
            &probe,
            || {
                let mut index = observed.borrow_mut();
                let pane = panes[(*index).min(panes.len() - 1)];
                *index += 1;
                Some(pane.to_string())
            },
            || may_press,
            || {
                *pressed.borrow_mut() += 1;
                Ok(())
            },
        );
        (result, *pressed.borrow(), *observed.borrow())
    }

    #[test]
    fn a_cleared_composer_confirms_the_first_enter() {
        let (result, pressed, _) = scripted(&[CLAUDE_PENDING, CLAUDE_SUBMITTED], true);
        assert_eq!(
            result.unwrap(),
            SubmitReport {
                outcome: SubmitOutcome::Submitted,
                enter_presses: 1
            }
        );
        assert_eq!(pressed, 1);
    }

    #[test]
    fn a_composer_that_never_clears_is_stuck_after_bounded_retries() {
        let (result, pressed, _) = scripted(&[CLAUDE_PENDING], true);
        assert_eq!(
            result.unwrap(),
            SubmitReport {
                outcome: SubmitOutcome::Stuck,
                enter_presses: MAX_ENTER_PRESSES
            }
        );
        assert_eq!(pressed, MAX_ENTER_PRESSES);
    }

    #[test]
    fn a_blocked_session_is_never_pressed_again() {
        let (result, pressed, _) = scripted(&[CLAUDE_PENDING], false);
        assert_eq!(
            result.unwrap(),
            SubmitReport {
                outcome: SubmitOutcome::Stuck,
                enter_presses: 1
            }
        );
        assert_eq!(pressed, 1);
    }

    #[test]
    fn a_dialog_after_enter_is_unverified_and_never_pressed() {
        let (result, pressed, _) = scripted(&[CLAUDE_PENDING, CLAUDE_TRUST_DIALOG], true);
        assert_eq!(
            result.unwrap(),
            SubmitReport {
                outcome: SubmitOutcome::Unverified,
                enter_presses: 1
            }
        );
        assert_eq!(pressed, 1);
    }

    #[test]
    fn a_clear_composer_without_the_text_ever_seen_is_unverified() {
        let (result, pressed, _) = scripted(&[CLAUDE_SUBMITTED], true);
        assert_eq!(result.unwrap().outcome, SubmitOutcome::Unverified);
        assert_eq!(pressed, 1);
    }

    #[test]
    fn an_unobservable_pane_gets_one_enter_and_no_wait() {
        let started = Instant::now();
        let (result, pressed, observed) = scripted(&["\n\n"], true);
        assert_eq!(result.unwrap().outcome, SubmitOutcome::Unverified);
        assert_eq!((pressed, observed), (1, 1));
        assert!(started.elapsed() < POLL_INTERVAL);
    }

    #[test]
    fn a_queued_prompt_is_reported_as_queued() {
        let (result, _, _) = scripted(&[CLAUDE_PENDING, CLAUDE_QUEUED], true);
        assert_eq!(result.unwrap().outcome, SubmitOutcome::Queued);
    }
}
