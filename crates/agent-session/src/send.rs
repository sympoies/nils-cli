//! Terminal send delivery; opt-in guards preserve unguarded delivery.
mod native;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command as ProcessCommand;
use std::thread;
use std::time::{Duration, Instant};

use jiff::{Timestamp, Zoned};
use serde::Serialize;
use serde_json::json;

use crate::{
    CliContext, CliError, ManualInputSection, PANE_INPUT_COMMAND_TIMEOUT,
    POST_PASTE_KEY_SETTLE_DELAY, PasteMode, SessionRecord, SpecialKey, acquire_session_record_lock,
    activity, auto_resume, capture_visible_pane_with_timeout, carries_literal_text, cli,
    codex_account, codex_app_server, delete_tmux_buffer, ensure_same_session_identity,
    exit_status_result, live_status, load_session_record, resolve_tmux_bin,
    run_output_with_timeout, run_status_with_timeout, send_guard, send_input_unlocked, send_submit,
    send_tmux_key, session_dir, write_private_file, write_session_record,
};

#[derive(Debug, Serialize)]
pub(crate) struct SendResult {
    pub(crate) id: String,
    pub(crate) tmux_session: String,
    pub(crate) sent_text: bool,
    pub(crate) keys: Vec<String>,
    /// Present when the send typed text and pressed exactly `enter`: what the
    /// pane showed after the submit. A stuck prompt is an error instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) submission: Option<send_submit::SubmitReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) guards: Option<send_guard::GuardReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) native: Option<native::Receipt>,
}

// Native dispatch keeps its short deadline so provider event waiters can run.
// Terminal admission and each guarded Enter require several tmux round trips;
// reserve those checks as well as the swallowed-Enter confirmation interval.
fn guarded_hold_budget(record: &SessionRecord) -> Duration {
    if codex_app_server::runtime_is_supported(record) {
        send_submit::HOLD_BUDGET
    } else {
        Duration::from_secs(15)
    }
}

pub(crate) fn send_to_session(
    context: &CliContext,
    mut args: cli::SendArgs,
) -> Result<SendResult, CliError> {
    // Picker shortcuts type into the terminal rather than paste. Never send
    // them into a working/unknown session, even with --allow-blocked.
    if args
        .keys
        .iter()
        .any(|key| matches!(key, SpecialKey::Printable(_)))
    {
        args.guards.require_idle = true;
    }
    let text = read_send_text(&args.text, args.text_stdin)?;
    if text.is_none() && args.keys.is_empty() {
        return Err(CliError::usage(
            "empty-send",
            "send requires --text, --text-stdin, or at least one --key",
            None,
        ));
    }
    let observed = load_session_record(context, &args.id)?;
    let tmux_bin = resolve_tmux_bin(args.tmux_bin.as_deref());
    let _trace =
        codex_app_server::guarded::trace::Scope::for_send(&observed, &args, text.as_deref());
    let record_lock = codex_app_server::guarded::trace::record_lock(|| {
        acquire_session_record_lock(context, &observed.id)
    })?;
    let guard_started = codex_app_server::guarded::trace::budget_start();
    let mut manual_input = ManualInputSection::new(record_lock);
    let mut record = load_session_record(context, &observed.id)?;
    ensure_same_session_identity(&observed, &record)?;
    if live_status(&tmux_bin, &record.tmux_session) != "running" {
        return Err(CliError::runtime(
            "session-not-running",
            format!("session is not running: {}", record.id),
            Some(json!({ "id": record.id })),
        ));
    }
    crate::restart::ensure_not_fenced(context, &record)?;
    if args.guards.enabled() {
        let admission = send_guard::Admission {
            context,
            record: &record,
            args: &args.guards,
            tmux_bin: &tmux_bin,
            verify_composer: carries_literal_text(text.as_deref()) && !args.keys.is_empty(),
            deadline: guard_started + guarded_hold_budget(&record),
        };
        admission.check_key()?;
        admission.check(None, "admission")?;
    }
    // Only input that types characters is gated. Special keys — including the
    // `enter` that confirms a highlighted choice — stay admitted, because
    // answering the dialog is the reason a blocked session remains addressable.
    // The check sits inside the record lock and the lock is held through the
    // pane write below, so the phase it refuses on is the phase the write would
    // have landed in.
    if carries_literal_text(text.as_deref()) && !args.allow_blocked {
        activity::refuse_if_blocked(
            context,
            &record,
            "answer the dialog with a special key, or resend with allow_blocked to type into it",
        )?;
    }
    let submits = codex_app_server::input_contains_submission(text.as_deref(), &args.keys);
    if submits {
        codex_app_server::ensure_manual_input_capability(context, &record)?;
        if !args.guards.enabled() {
            codex_account::authorize_terminal_input_locked(context, &mut record)?;
        }
    }
    if args.guards.enabled() {
        return guarded(
            context,
            args,
            text,
            record,
            &tmux_bin,
            &mut manual_input,
            guard_started,
        );
    }
    auto_resume::cancel_for_manual_input_locked(
        context,
        &record.id,
        &Timestamp::now().to_string(),
    )?;
    // Typed text followed by exactly one Enter is a prompt submission. Its
    // Enter is pressed by the confirmation loop, which reads the composer back
    // and retries a swallowed Enter, so the caller learns whether the prompt
    // left the composer instead of assuming it did.
    let submits_prompt = carries_literal_text(text.as_deref()) && args.keys == [SpecialKey::Enter];
    let keys_after_paste: &[SpecialKey] = if submits_prompt { &[] } else { &args.keys };
    let hold_started = Instant::now();
    send_input_unlocked(
        context,
        &record,
        text.as_deref(),
        keys_after_paste,
        &tmux_bin,
        Some(&mut manual_input),
        PasteMode::Bracketed,
    )?;
    let submission = if submits_prompt {
        thread::sleep(POST_PASTE_KEY_SETTLE_DELAY);
        Some(confirm_prompt_submission(
            context,
            &record,
            text.as_deref().unwrap_or_default(),
            &tmux_bin,
            &mut manual_input,
            hold_started,
        )?)
    } else {
        None
    };
    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, &record)?;
    if let Some(report) = submission
        && report.outcome == send_submit::SubmitOutcome::Stuck
    {
        return Err(CliError::runtime(
            "send-submit-stuck",
            format!(
                "the text is still in the session's input box after {} enter presses: {}",
                report.enter_presses, record.id
            ),
            Some(json!({
                "id": record.id,
                "outcome": report.outcome,
                "enter_presses": report.enter_presses,
            })),
        ));
    }
    Ok(SendResult {
        id: record.id.clone(),
        tmux_session: record.tmux_session.clone(),
        sent_text: text.is_some(),
        keys: args
            .keys
            .iter()
            .map(|key| key.as_str().to_string())
            .collect(),
        submission,
        guards: None,
        native: None,
    })
}

/// Press the submitting Enter for pasted `text` and confirm from the pane that
/// it left the composer. Only Claude Code and Codex composers are recognized;
/// any other provider gets the single Enter and an `unverified` outcome.
fn confirm_prompt_submission(
    context: &CliContext,
    record: &SessionRecord,
    text: &str,
    tmux_bin: &Path,
    manual_input: &mut ManualInputSection,
    hold_started: Instant,
) -> Result<send_submit::SubmitReport, CliError> {
    let target = format!("{}:0.0", record.tmux_session);
    let mut press_enter = || {
        manual_input.arm(context, record)?;
        send_tmux_key(tmux_bin, &target, SpecialKey::Enter)
    };
    let Some(probe) = send_submit::probe(text) else {
        press_enter()?;
        return Ok(send_submit::SubmitReport {
            outcome: send_submit::SubmitOutcome::Unverified,
            enter_presses: 1,
        });
    };
    send_submit::submit_and_confirm(
        &record.agent,
        &probe,
        hold_started + send_submit::HOLD_BUDGET,
        |timeout| capture_visible_pane_with_timeout(tmux_bin, &target, timeout),
        |timeout| {
            crate::live_status_with_timeout(tmux_bin, &record.tmux_session, timeout) == "stopped"
        },
        || {
            activity::state_for_view(context, record)
                .is_none_or(|turn| turn.phase != activity::TurnPhase::NeedsInput)
        },
        press_enter,
    )
}

pub(crate) fn load_and_paste_buffer_with_timeout(
    tmux_bin: &Path,
    buffer_name: &str,
    target: &str,
    file: &Path,
    timeout: Duration,
    paste_mode: PasteMode,
) -> Result<(), CliError> {
    load_and_paste_guarded(
        tmux_bin,
        buffer_name,
        target,
        file,
        timeout,
        paste_mode,
        || Ok(()),
    )
}

pub(crate) fn load_and_paste_guarded(
    tmux_bin: &Path,
    buffer_name: &str,
    target: &str,
    file: &Path,
    timeout: Duration,
    paste_mode: PasteMode,
    admit: impl FnOnce() -> Result<(), CliError>,
) -> Result<(), CliError> {
    let mut load = ProcessCommand::new(tmux_bin);
    load.arg("load-buffer").arg("-b").arg(buffer_name).arg(file);
    run_status_with_timeout(load, "tmux load-buffer", timeout)?;

    if let Err(err) = admit() {
        delete_tmux_buffer(tmux_bin, buffer_name);
        return Err(err);
    }

    let paste = |raw_bytes: bool| {
        let mut paste = ProcessCommand::new(tmux_bin);
        paste.arg("paste-buffer");
        if paste_mode == PasteMode::Bracketed {
            paste.arg("-p");
        }
        // tmux 3.7 passes paste buffers through vis(3) unless `-S` is given,
        // so forwarded terminal input (a mouse report, Alt-key, or Esc) would
        // reach the pane as literal `^[` text.
        if raw_bytes {
            paste.arg("-S");
        }
        paste
            .arg("-b")
            .arg(buffer_name)
            .arg("-d")
            .arg("-t")
            .arg(target);
        paste
    };
    let label = "tmux paste-buffer";
    let result = if paste_mode == PasteMode::Raw {
        match run_output_with_timeout(paste(true), timeout) {
            // tmux before 3.7 has no `-S` and does not sanitize pastes.
            Ok(output)
                if !output.status.success()
                    && String::from_utf8_lossy(&output.stderr).contains("unknown flag -S") =>
            {
                run_status_with_timeout(paste(false), label, timeout)
            }
            output => exit_status_result(output.map(|output| output.status), label),
        }
    } else {
        run_status_with_timeout(paste(false), label, timeout)
    };
    if let Err(err) = result {
        delete_tmux_buffer(tmux_bin, buffer_name);
        return Err(err);
    }
    Ok(())
}

fn read_send_text(text: &Option<String>, text_stdin: bool) -> Result<Option<String>, CliError> {
    // Empty text (an empty `--text ""` or an empty stdin pipe) collapses to
    // `None` so the caller's empty-send guard treats it as "no text" rather than
    // pasting an empty buffer and reporting `sent_text: true` for a no-op. A
    // whitespace-only value is preserved: a space can be a meaningful keystroke.
    match (text, text_stdin) {
        (Some(_), true) => Err(CliError::usage(
            "multiple-text-sources",
            "use only one of --text or --text-stdin",
            None,
        )),
        (Some(value), false) => Ok(Some(value.clone()).filter(|value| !value.is_empty())),
        (None, true) => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).map_err(|err| {
                CliError::runtime(
                    "stdin-read-failed",
                    format!("failed to read stdin: {err}"),
                    None,
                )
            })?;
            Ok(Some(input).filter(|value| !value.is_empty()))
        }
        (None, false) => Ok(None),
    }
}

pub(crate) fn unguarded_args(
    id: String,
    text: Option<String>,
    keys: Vec<SpecialKey>,
    allow_blocked: bool,
    tmux_bin: std::path::PathBuf,
) -> cli::SendArgs {
    cli::SendArgs {
        id,
        text,
        text_stdin: false,
        keys,
        allow_blocked,
        tmux_bin: Some(tmux_bin),
        format: nils_common::cli_contract::OutputFormat::Json,
        guards: send_guard::GuardArgs::default(),
    }
}

fn guarded(
    context: &CliContext,
    args: cli::SendArgs,
    text: Option<String>,
    mut record: SessionRecord,
    tmux_bin: &Path,
    manual_input: &mut ManualInputSection,
    hold_started: Instant,
) -> Result<SendResult, CliError> {
    let observed = record.clone();
    let admission = send_guard::Admission {
        context,
        record: &observed,
        args: &args.guards,
        tmux_bin,
        verify_composer: carries_literal_text(text.as_deref()) && !args.keys.is_empty(),
        deadline: hold_started + guarded_hold_budget(&record),
    };
    admission.check_key()?;
    admission.check(None, "admission")?;
    if codex_app_server::runtime_is_supported(&record)
        && args.guards.require_idle
        && args.guards.require_empty_composer
        && carries_literal_text(text.as_deref())
        && args.keys == [SpecialKey::Enter]
    {
        return native::deliver(
            context,
            args,
            text.expect("literal native input"),
            record,
            tmux_bin,
            manual_input,
            hold_started,
        );
    }
    let submits = codex_app_server::input_contains_submission(text.as_deref(), &args.keys);
    let mut input_prepared = false;
    let target = format!("{}:0.0", record.tmux_session);
    let literal = match text
        .as_deref()
        .filter(|_| carries_literal_text(text.as_deref()))
    {
        Some(text) => Some(admission.observable_text(text)?),
        None => None,
    };
    if let Some(text) = literal {
        let nonce = uuid::Uuid::new_v4();
        let buffer = format!("{}-send-{nonce}", record.id);
        let temp = session_dir(context, &record.id).join(format!("send-input-{nonce}"));
        write_private_file(&temp, text.as_bytes())?;
        let delivered = load_and_paste_guarded(
            tmux_bin,
            &buffer,
            &target,
            &temp,
            PANE_INPUT_COMMAND_TIMEOUT,
            PasteMode::Bracketed,
            || {
                admission.check(None, "admission")?;
                admission.claim_key()?;
                admission.check(None, "admission")?;
                prepare_guarded_input(context, &mut record, submits, &mut input_prepared)
            },
        );
        let _ = fs::remove_file(temp);
        delivered?;
    } else {
        admission.claim_key()?;
        admission.check(None, "admission")?;
    }
    // Mixed-prefix recovery needs an exact-paste observation before a conflict.
    let mut observed_paste = false;
    // From here the pasted text sits in the input: a refusal must report it.
    let mut after_paste = || -> Result<Option<send_submit::SubmitReport>, CliError> {
        if let Some(text) = literal
            && !args.keys.is_empty()
        {
            observed_paste = admission.settle_paste(text);
        }
        let submits_prompt = literal.is_some() && args.keys == [SpecialKey::Enter];
        if submits_prompt {
            let text = literal.expect("literal prompt");
            let probe = send_submit::probe(text);
            let mut press_enter = || {
                manual_input.arm(context, &record)?;
                admission.check(Some(text), "submit")?;
                send_tmux_key(tmux_bin, &target, SpecialKey::Enter)
            };
            if let Some(probe) = probe {
                Ok(Some(send_submit::submit_and_confirm(
                    &record.agent,
                    &probe,
                    admission.deadline,
                    |timeout| capture_visible_pane_with_timeout(tmux_bin, &target, timeout),
                    |timeout| {
                        crate::live_status_with_timeout(tmux_bin, &record.tmux_session, timeout)
                            == "stopped"
                    },
                    || true,
                    press_enter,
                )?))
            } else {
                press_enter()?;
                Ok(Some(send_submit::SubmitReport {
                    outcome: send_submit::SubmitOutcome::Unverified,
                    enter_presses: 1,
                }))
            }
        } else {
            if text.as_deref().is_some_and(crate::text_is_bare_newline) {
                manual_input.arm(context, &record)?;
                admission.check(None, "submit")?;
                prepare_guarded_input(context, &mut record, submits, &mut input_prepared)?;
                send_tmux_key(tmux_bin, &target, SpecialKey::Enter)?;
            }
            for key in &args.keys {
                if *key == SpecialKey::Enter {
                    manual_input.arm(context, &record)?;
                }
                admission.check(literal, "submit")?;
                prepare_guarded_input(context, &mut record, submits, &mut input_prepared)?;
                send_tmux_key(tmux_bin, &target, *key)?;
            }
            Ok(None)
        }
    };
    let submission = match (after_paste(), literal) {
        (Ok(submission), _) => submission,
        (Err(error), Some(text)) => {
            return Err(admission.report_pasted_input(error, text, observed_paste));
        }
        (Err(error), None) => return Err(error),
    };
    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, &record)?;
    if let Some(report) = submission
        && matches!(
            report.outcome,
            send_submit::SubmitOutcome::Stuck | send_submit::SubmitOutcome::Unverified
        )
    {
        let error = CliError::runtime(
            if report.outcome == send_submit::SubmitOutcome::Stuck {
                "send-submit-stuck"
            } else {
                "send-submit-unconfirmed"
            },
            "guarded prompt submission was not confirmed; the delivery claim is retained",
            Some(json!({"id": record.id, "outcome": report.outcome,
                "enter_presses": report.enter_presses, "atomic": false})),
        );
        return Err(admission.report_pasted_input(
            error,
            literal.expect("unconfirmed literal prompt"),
            observed_paste,
        ));
    }
    Ok(SendResult {
        id: record.id,
        tmux_session: record.tmux_session,
        sent_text: text.is_some(),
        keys: args
            .keys
            .iter()
            .map(|key| key.as_str().to_string())
            .collect(),
        submission,
        guards: Some(send_guard::GuardReport { atomic: false }),
        native: None,
    })
}

/// Publish control state only after final admission, immediately before input.
/// Once input may have reached the pane, retain these effects on later refusal.
fn prepare_guarded_input(
    context: &CliContext,
    record: &mut SessionRecord,
    submits: bool,
    prepared: &mut bool,
) -> Result<(), CliError> {
    if !*prepared {
        if submits {
            codex_account::authorize_terminal_input_locked(context, record)?;
        }
        auto_resume::cancel_for_manual_input_locked(
            context,
            &record.id,
            &Timestamp::now().to_string(),
        )?;
        *prepared = true;
    }
    Ok(())
}
