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

/// Leading non-whitespace characters used to find the text again. Short enough
/// to stay ahead of any provider rewriting further into a long text, long enough
/// not to match a placeholder by accident.
const PROBE_CHARS: usize = 20;
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// How long the text may stay in the composer after an Enter before that Enter
/// counts as swallowed.
const RETRY_AFTER: Duration = Duration::from_millis(1500);
/// The caller's Enter plus one retry for the swallowed first Enter.
const MAX_ENTER_PRESSES: u32 = 2;
/// Upper bound on the record-lock hold from the paste to the last pane read,
/// including the caller's settle delay (the tmux paste and Enter commands keep
/// their own timeouts). `send` holds the session record lock throughout, and
/// Codex app-server attention events and turn-complete notifications that
/// find it busy wait only five seconds, so the hold must end well before that.
pub(crate) const HOLD_BUDGET: Duration = Duration::from_millis(4000);
/// Longest single pane read; a read never runs past the hold budget either.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(1);
/// Shortest time a pane read is given. With less left in the budget the loop
/// stops instead of starting a read that a slow host could not finish.
const MIN_CAPTURE_TIME: Duration = Duration::from_millis(300);
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

/// What the composer must show while it still holds the sent text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    /// Whitespace-free text to find at the start of the composer input.
    head: String,
    /// `head` is the whole text, so the input must be exactly `head`. A short
    /// reply such as `Ask` would otherwise match the "Ask Codex to do
    /// anything" placeholder of the composer it leaves empty.
    whole: bool,
}

/// The whitespace-free start of the text, or `None` when the text has nothing
/// to find again. Line breaks are dropped with the rest of the whitespace, so a
/// probe spans lines exactly as the compacted composer region does.
pub(crate) fn probe(text: &str) -> Option<Probe> {
    let all = compact(text);
    if all.is_empty() {
        return None;
    }
    Some(Probe {
        whole: all.chars().count() <= PROBE_CHARS,
        head: all.chars().take(PROBE_CHARS).collect(),
    })
}

/// Whitespace is dropped before matching so a wrapped line and its
/// continuation indent still match the probe.
fn compact(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

pub(crate) fn classify(agent: &str, pane: &str, probe: &Probe) -> Composer {
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
    let input = region.trim_start_matches(['❯', '›']);
    // An empty composer shows a placeholder ("Ask Codex to do anything"), so a
    // text no longer than the probe must be the whole input. A full-length
    // probe may sit after an existing draft.
    let holds_text = if probe.whole {
        input == probe.head
    } else {
        input.contains(&probe.head)
    } || PASTE_PLACEHOLDERS.iter().any(|mark| input.contains(mark));
    if agent == "claude" && input == CLAUDE_QUEUED_HINT {
        Composer::Queued
    } else if holds_text {
        Composer::Pending
    } else {
        Composer::Clear
    }
}

fn claude_rules(lines: &[&str]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.trim_start().starts_with('─') && line.chars().filter(|ch| *ch == '─').count() >= 10
        })
        .map(|(index, _)| index)
        .collect()
}

/// Whether a Claude pane shows the sent text accepted into its queue: the
/// composer shows the queued hint and the queued messages above it include the
/// start of the text. The hint alone also appears for an earlier message.
pub(crate) fn claude_queue_holds(pane: &str, probe: &Probe) -> bool {
    if classify("claude", pane, probe) != Composer::Queued {
        return false;
    }
    let lines: Vec<&str> = pane.lines().map(str::trim_end).collect();
    let rules = claude_rules(&lines);
    let [.., top, _] = rules[..] else {
        return false;
    };
    compact(&lines[..top].join("\n")).contains(&probe.head)
}

/// Claude Code draws its input box between the last two horizontal rules and
/// starts it with `❯`. A permission or trust dialog replaces the box, so no
/// such region exists while one is open.
fn claude_composer<'a>(lines: &'a [&'a str]) -> Option<(&'a [&'a str], &'a [&'a str])> {
    let rules = claude_rules(lines);
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
/// lines, followed by a blank line and a short status footer. A pasted
/// paragraph break is a blank line inside the composer, so the footer is the
/// block after the last blank line. Its transcript echoes earlier prompts with
/// the same `›`, so a `›` block followed by more than a footer, or by
/// unindented output, is transcript, not the composer.
fn codex_composer<'a>(lines: &'a [&'a str]) -> Option<(&'a [&'a str], &'a [&'a str])> {
    let start = lines.iter().rposition(|line| line.starts_with('›'))?;
    if starts_with_numbered_choice(&lines[start]['›'.len_utf8()..]) {
        return None;
    }
    let mut end = lines.len();
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let below = &lines[start + 1..end];
    let region_end = match below.iter().rposition(|line| line.trim().is_empty()) {
        Some(blank) => start + 1 + blank,
        None => below
            .iter()
            .position(|line| !line.starts_with("  "))
            .map_or(end, |offset| start + 1 + offset),
    };
    let region = &lines[start..region_end];
    let continuation_indented = region[1..]
        .iter()
        .all(|line| line.trim().is_empty() || line.starts_with("  "));
    let footer = &lines[region_end..];
    let footer_lines = footer.iter().filter(|line| !line.trim().is_empty()).count();
    (continuation_indented && footer_lines <= CODEX_MAX_FOOTER_LINES).then_some((region, footer))
}

/// Provider selection lists draw their cursor as the composer glyph followed by
/// `1.`, `2.` …; such a line is a choice, not typed input.
fn starts_with_numbered_choice(after_glyph: &str) -> bool {
    let rest = after_glyph.trim_start_matches([' ', '\u{a0}']);
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with('.')
}

/// Press the submitting Enter, then watch the composer until it proves the
/// outcome within [`HOLD_BUDGET`] of `hold_started`, the moment the text was
/// pasted. `observe` returns the visible pane read within the given timeout
/// (`None` when tmux cannot answer),
/// `may_press` is consulted before every retry so a session recorded as waiting
/// on a dialog is never answered, and `press_enter` sends one Enter. Provider
/// events cannot land while the caller holds the session lock, so the pane
/// classifier, which never sees a composer behind a dialog, is the primary
/// guard and `may_press` covers state recorded before the send.
pub(crate) fn submit_and_confirm(
    agent: &str,
    probe: &Probe,
    hold_started: Instant,
    mut observe: impl FnMut(Duration) -> Option<String>,
    mut may_press: impl FnMut() -> bool,
    mut press_enter: impl FnMut() -> Result<(), crate::CliError>,
) -> Result<SubmitReport, crate::CliError> {
    let view = |pane: Option<String>| {
        pane.map_or(Composer::Unrecognized, |pane| classify(agent, &pane, probe))
    };
    let deadline = hold_started + HOLD_BUDGET;
    let remaining = || deadline.saturating_duration_since(Instant::now());
    // A blank capture is a pane tmux could not show us, not an empty composer.
    let before = observe(remaining().min(CAPTURE_TIMEOUT)).filter(|pane| !pane.trim().is_empty());
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
    // The verdict at the end of the budget rests on the last pane tmux actually
    // returned; a read that timed out proves nothing about the composer.
    let mut last_view = Composer::Unrecognized;
    loop {
        if remaining() <= POLL_INTERVAL + MIN_CAPTURE_TIME {
            let outcome = if last_view == Composer::Pending {
                SubmitOutcome::Stuck
            } else {
                SubmitOutcome::Unverified
            };
            return Ok(report(outcome, presses));
        }
        thread::sleep(POLL_INTERVAL);
        let Some(pane) = observe(remaining().min(CAPTURE_TIMEOUT)) else {
            continue;
        };
        let current = classify(agent, &pane, probe);
        last_view = current;
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
            Composer::Unrecognized
                if seen_pending && unrecognized_polls >= UNRECOGNIZED_POLLS_AFTER_PENDING =>
            {
                return Ok(report(SubmitOutcome::Unverified, presses));
            }
            Composer::Clear | Composer::Unrecognized => {}
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

    fn probe_of(text: &str) -> Probe {
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
    fn a_short_probe_is_not_found_inside_an_empty_composer_placeholder() {
        for text in ["y", "n", "do", "fix", "Ask", "A", "Try"] {
            assert_eq!(
                classify("codex", CODEX_SUBMITTED, &probe_of(text)),
                Composer::Clear,
                "{text}"
            );
            assert_eq!(
                classify("claude", CLAUDE_STARTUP_PLACEHOLDER, &probe_of(text)),
                Composer::Clear,
                "{text}"
            );
        }
        assert_eq!(
            classify("claude", CLAUDE_QUEUED, &probe_of("up")),
            Composer::Queued
        );
        let pending = "────────────────────\n❯\u{a0}y\n────────────────────\n";
        assert_eq!(
            classify("claude", pending, &probe_of("y")),
            Composer::Pending
        );
        let multi = "────────────────────\n❯\u{a0}ok\n  go\n────────────────────\n";
        assert_eq!(
            classify("claude", multi, &probe_of("ok\ngo")),
            Composer::Pending
        );
    }

    #[test]
    fn a_codex_prompt_with_a_paragraph_break_stays_one_composer() {
        let pane = "› Earlier prompt\n\n• Working (3s • esc to interrupt)\n\n› Release notes\n  \n  first body line\n  second body line\n  third body line\n  fourth body line\n\n  fake-model · Context 100% left\n";
        assert_eq!(
            classify(
                "codex",
                pane,
                &probe_of("Release notes\n\nfirst body line\nsecond body line")
            ),
            Composer::Pending
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
            Probe {
                head: "firstlinethatislongenoughtocut"[..20].to_string(),
                whole: false
            }
        );
        assert_eq!(
            probe_of("Ask\nfor the deploy status of every host"),
            Probe {
                head: "Askforthedeploystatu".to_string(),
                whole: false
            }
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
            Instant::now(),
            |_| {
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
    fn slow_pane_reads_never_hold_the_lock_past_the_budget() {
        let hold_started = Instant::now();
        let presses = RefCell::new(0u32);
        let result = submit_and_confirm(
            "claude",
            &probe_of("[Monitoring] REPRO A1: Disk usage high"),
            hold_started,
            |timeout| {
                thread::sleep(timeout.min(Duration::from_millis(900)));
                Some(CLAUDE_PENDING.to_string())
            },
            || true,
            || {
                *presses.borrow_mut() += 1;
                Ok(())
            },
        );
        assert_eq!(result.unwrap().outcome, SubmitOutcome::Stuck);
        assert!(
            hold_started.elapsed() < HOLD_BUDGET + POLL_INTERVAL,
            "held for {:?}",
            hold_started.elapsed()
        );
    }

    #[test]
    fn pane_reads_that_time_out_do_not_turn_a_stuck_prompt_unverified() {
        let reads = RefCell::new(0u32);
        let result = submit_and_confirm(
            "claude",
            &probe_of("[Monitoring] REPRO A1: Disk usage high"),
            Instant::now(),
            |_| {
                let mut reads = reads.borrow_mut();
                *reads += 1;
                // A slow host answers the first reads and then times out.
                (*reads <= 4).then(|| CLAUDE_PENDING.to_string())
            },
            || true,
            || Ok(()),
        );
        assert_eq!(result.unwrap().outcome, SubmitOutcome::Stuck);
    }

    #[test]
    fn a_composer_never_seen_gets_the_full_grace_before_unverified() {
        let started = Instant::now();
        let (result, pressed, _) = scripted(&[CLAUDE_TRUST_DIALOG], true);
        assert_eq!(result.unwrap().outcome, SubmitOutcome::Unverified);
        assert_eq!(pressed, 1);
        assert!(started.elapsed() >= RETRY_AFTER);
    }

    #[test]
    fn a_queue_holds_only_the_text_listed_above_the_composer() {
        assert!(claude_queue_holds(
            CLAUDE_QUEUED,
            &probe_of("Second queued message.")
        ));
        assert!(!claude_queue_holds(
            CLAUDE_QUEUED,
            &probe_of("A different prompt")
        ));
        assert!(!claude_queue_holds(
            CLAUDE_PENDING,
            &probe_of("Second queued message.")
        ));
    }

    #[test]
    fn a_queued_prompt_is_reported_as_queued() {
        let (result, _, _) = scripted(&[CLAUDE_PENDING, CLAUDE_QUEUED], true);
        assert_eq!(result.unwrap().outcome, SubmitOutcome::Queued);
    }
}
