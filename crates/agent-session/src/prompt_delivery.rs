//! Shared staged paste delivery and request-bound Claude composer evidence.
use super::*;

/// Deliver one staged prompt file into the session pane and submit it. The
/// caller owns the file so that both session startup and the post-start
/// structured prompt route share exactly one hardened delivery, and it owns the
/// pane-ready deadline because a request-bound caller must answer its client
/// sooner than a launch may wait.
pub(crate) fn paste_prompt_file(
    tmux_bin: &Path,
    record: &SessionRecord,
    prompt_file: &Path,
    pane_ready_deadline: Duration,
) -> Result<PromptDeliveryObservation, CliError> {
    paste_prompt_file_observing(
        tmux_bin,
        record,
        prompt_file,
        pane_ready_deadline,
        None,
        None,
        None,
    )
}

/// Observes the pane before Enter and returns the composer the provider rendered.
type ComposerRendering<'a> = dyn FnMut(&str) -> Option<String> + 'a;

fn paste_prompt_file_observing(
    tmux_bin: &Path,
    record: &SessionRecord,
    prompt_file: &Path,
    pane_ready_deadline: Duration,
    before_enter: Option<&mut ComposerRendering<'_>>,
    response_context: Option<(&CliContext, &str)>,
    evidence_out: Option<&mut Option<send::stranded::Record>>,
) -> Result<PromptDeliveryObservation, CliError> {
    let buffer_name = format!("{}-prompt", record.id);
    let target = format!("{}:0.0", record.tmux_session);

    // The caller's fixed pre-paste delay is only a guess at provider startup.
    // Confirm the pane has actually been drawn before pasting: bytes delivered
    // while the launch shell still owns the tty are echoed into scrollback and
    // lost, and the single-Enter recovery cannot restore a prompt that never
    // reached the provider's input at all.
    await_pane_drawn_before_paste(tmux_bin, &target, pane_ready_deadline);

    // Observe rendering for diagnostics only. An accepted paste may still be
    // queued while the visible pane stays unchanged; no pane digest can prove
    // non-delivery or authorize replaying the prompt bytes.
    let before = capture_pane_digest(tmux_bin, &target);
    // One admission spans the paste and its Enter. A hook writer that appears
    // after the paste waits on this fence instead of refusing the Enter and
    // leaving the text in the composer. The fence is retained only when the
    // admission is for a Claude pane; provider events resume when it drops.
    let _ingress_hold = response_context
        .map(|(context, text)| {
            let fence =
                cli::permission::input_admission(context, record, tmux_bin, Some(text), &[])?;
            Ok::<_, CliError>(
                fence.map(|fence| activity::pending::hold(context, &record.id, fence)),
            )
        })
        .transpose()?
        .flatten();
    let mut evidence = response_context
        .filter(|_| record.agent == "claude")
        .map(|(context, text)| send::stranded::begin(context, record, text))
        .transpose()?;
    paste::load_and_paste_buffer(tmux_bin, &buffer_name, &target, prompt_file, record)?;
    if let (Some(evidence), Some((context, _))) = (evidence.as_mut(), response_context) {
        if let Err(mut error) = evidence.pasted(context, record) {
            *error.details_mut() = Some(serde_json::json!({
                "pasted": true, "cleared": false, "remaining_input": "unobservable"
            }));
            post_paste::record_failure(context, record, &error);
            return Err(error);
        }
    }

    // The initial prompt is submitted; `send` deliberately leaves this to
    // an explicit `--key enter`. Tmux confirms that it wrote the paste bytes,
    // but provider TUIs can still be processing the bracketed paste when an
    // immediately adjacent Enter arrives. Keep the key a separate command and
    // give both Codex and Claude one bounded settle interval first.
    thread::sleep(POST_PASTE_KEY_SETTLE_DELAY);
    let after = capture_pane_digest(tmux_bin, &target);
    let composer = PromptComposerObservation {
        state: match (before, after) {
            (Some(before), Some(after)) if before == after => "unchanged_after_paste",
            (Some(_), Some(_)) => "changed_after_paste",
            _ => "unavailable",
        },
        proof: "tmux-pane-digest-before-submit",
    };
    let submitted = (|| {
        let mut represented_text = response_context.map(|(_, text)| text.to_owned());
        if let Some(observe) = before_enter
            && let Some(pane) = capture_visible_pane(tmux_bin, &target)
            && let Some(rendered) = observe(&pane)
        {
            represented_text = Some(rendered);
        }
        if let Some((context, text)) = response_context
            && record.agent == "claude"
        {
            let report = send_guard::placeholder::submit_after_empty(
                context,
                record,
                represented_text.as_deref().unwrap_or(text),
                tmux_bin,
                Instant::now() + send::guarded_hold_budget(record, false),
                || {
                    send_guard::response::watch(
                        context,
                        record,
                        tmux_bin,
                        send_guard::response::Operation::Submit,
                        || {
                            let _permission_admission = cli::permission::input_admission(
                                context,
                                record,
                                tmux_bin,
                                Some(text),
                                &[SpecialKey::Enter],
                            )?;
                            send_tmux_key(tmux_bin, &target, SpecialKey::Enter)
                        },
                    )
                },
            )?;
            if report.outcome == send_submit::SubmitOutcome::Stuck {
                return Err(CliError::runtime(
                    "send-submit-stuck",
                    "prompt submission was not confirmed",
                    Some(json!({"id":record.id,"outcome":report.outcome,
                        "enter_presses":report.enter_presses,"atomic":false,
                        "pasted":true,"cleared":false,"remaining_input":"unobservable"})),
                ));
            }
            if matches!(
                report.outcome,
                send_submit::SubmitOutcome::Submitted | send_submit::SubmitOutcome::Queued
            ) && let Some(evidence) = evidence.as_mut()
            {
                evidence.submitted(context, record)?;
            }
            return Ok(());
        }
        let mut enter = ProcessCommand::new(tmux_bin);
        enter.arg("send-keys").arg("-t").arg(&target).arg("Enter");
        if let Some((context, _)) = response_context {
            send_guard::response::watch(
                context,
                record,
                tmux_bin,
                send_guard::response::Operation::Submit,
                || run_status(enter, "tmux send-keys"),
            )
        } else {
            run_status(enter, "tmux send-keys")
        }
    })();
    if let (Err(error), Some((context, _))) = (&submitted, response_context) {
        post_paste::record_failure(context, record, error);
    }
    submitted?;
    if let Some(output) = evidence_out {
        *output = evidence;
    }
    Ok(PromptDeliveryObservation {
        schema_version: "agent-session.prompt-delivery-observation.v1",
        composer,
    })
}

/// Paste one structured prompt into a Claude pane and submit it while the
/// session record lock fences the exact runtime, returning the incarnation the
/// text was written to and its observed provider representation. The staged
/// file keeps the prompt out of argv, and the
/// shared start-path delivery keeps the pane-readiness wait and delivers the
/// prompt bytes once, even when rendering or acknowledgement is uncertain.
pub(crate) fn deliver_claude_structured_prompt_locked(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    text: &str,
    tmux_bin: &Path,
) -> Result<ClaudePromptDelivery, CliError> {
    let _record_lock = acquire_session_record_lock(context, id)?;
    let mut record = load_session_record(context, id)?;
    let actual_launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .filter(|launch_id| !launch_id.trim().is_empty());
    if actual_launch_id.as_deref() != Some(expected_launch_id) {
        return Err(crate::serve::structured_prompt_incarnation_conflict(
            id,
            expected_launch_id,
            actual_launch_id.as_deref(),
        ));
    }
    if live_status(tmux_bin, &record.tmux_session) != "running" {
        return Err(CliError::runtime(
            "session-not-running",
            format!("session is not running: {}", record.id),
            Some(json!({ "id": record.id })),
        ));
    }
    crate::auth_incident::recovery::ensure_input_allowed(context, &record)?;
    crate::restart::ensure_not_fenced(context, &record)?;
    // Recheck blocked activity under the runtime lock before touching input.
    activity::refuse_if_blocked(
        context,
        &record,
        "resolve the pending dialog, then resubmit the prompt",
    )?;
    send_guard::require_empty_composer(context, &record, tmux_bin)?;
    codex_account::ensure_input_allowed(&record)?;
    auto_resume::cancel_for_manual_input_locked(
        context,
        &record.id,
        &Timestamp::now().to_string(),
    )?;

    // Each proof starts under the same lock as its own paste. Concurrent
    // requests must not share evidence produced by the earlier lock holder.
    let turn_baseline = activity::prompt_turn_baseline(context, &record);
    let prompt_tail = crate::provider_prompt::ProviderPromptTail::resolve_source(&record)
        .and_then(crate::provider_prompt::ProviderPromptTail::open_source_at_eof);
    let queue_before = capture_visible_pane(tmux_bin, &format!("{}:0.0", record.tmux_session));

    let nonce = uuid::Uuid::new_v4();
    let staged = session_dir(context, &record.id).join(format!("structured-prompt-{nonce}"));
    write_private_file(&staged, text.as_bytes())?;
    let mut acknowledgement_text = text.to_string();
    let image_prompt = image_prompt(text, &record.cwd);
    let mut observe = |pane: &str| {
        let (count, rest) = image_prompt.as_ref()?;
        let rendered = composer_image_label(pane, *count, rest)?;
        acknowledgement_text = rendered.clone();
        Some(rendered)
    };
    let mut stranded_evidence = None;
    let delivered = paste_prompt_file_observing(
        tmux_bin,
        &record,
        &staged,
        CLAUDE_STRUCTURED_PROMPT_PANE_READY_DEADLINE,
        image_prompt
            .is_some()
            .then_some(&mut observe as &mut ComposerRendering<'_>),
        Some((context, text)),
        Some(&mut stranded_evidence),
    );
    let _ = fs::remove_file(&staged);
    delivered?;

    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, &record)?;
    Ok(ClaudePromptDelivery {
        session_incarnation: expected_launch_id.to_string(),
        acknowledgement_text,
        turn_baseline,
        prompt_tail,
        queue_before,
        stranded_evidence,
    })
}

/// Transient matching text is never persisted or exposed as receipt metadata.
pub(crate) struct ClaudePromptDelivery {
    pub(crate) session_incarnation: String,
    pub(crate) acknowledgement_text: String,
    pub(crate) turn_baseline: activity::PromptTurnBaseline,
    pub(crate) prompt_tail: Option<crate::provider_prompt::ProviderPromptTail>,
    pub(crate) queue_before: Option<String>,
    pub(crate) stranded_evidence: Option<send::stranded::Record>,
}

/// Only a single existing image path may be rewritten to an observed image
/// label. Never guess a label from a counter or accept arbitrary changed text.
fn image_path_prompt(text: &str, cwd: &str) -> bool {
    let text = text.trim();
    let path = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(text);
    if path.contains(['\n', '\r', '"', '\'']) {
        return false;
    }
    let path = Path::new(cwd).join(path);
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["png", "jpg", "jpeg", "gif", "webp"]
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
        && path.is_file()
}

/// The leading image paths of a prompt and the text after them. Claude replaces
/// each path with one image label and keeps the rest of the text. Further paths
/// must be quoted existing images, as the first one may be unquoted only on a
/// line of its own.
fn image_prompt(text: &str, cwd: &str) -> Option<(usize, String)> {
    let mut rest = text.trim_start();
    let mut count = 0;
    loop {
        let (path, after) = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => match rest[1..].find(quote) {
                Some(end) => rest.split_at(end + 2),
                None => break,
            },
            Some(_) if count == 0 => rest.split_once('\n').unwrap_or((rest, "")),
            _ => break,
        };
        if !image_path_prompt(path, cwd) {
            break;
        }
        count += 1;
        rest = after.trim_start();
    }
    (count > 0).then(|| (count, rest.trim().to_owned()))
}

/// The observed composer, when it is exactly `count` consecutive image labels
/// followed by the prompt's remaining text. Whitespace between them is the
/// renderer's choice.
fn composer_image_label(pane: &str, count: usize, rest: &str) -> Option<String> {
    let input = send_submit::exact_composer_input("claude", pane)?;
    let mut shown = input.as_str();
    let mut previous: Option<u32> = None;
    for _ in 0..count {
        let (number, after) = shown
            .trim_start()
            .strip_prefix("[Image #")?
            .split_once(']')?;
        let number = (number.len() <= 10 && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| number.parse::<u32>().ok())
            .flatten()
            .filter(|n| *n > 0)?;
        if previous.is_some_and(|last| last.checked_add(1) != Some(number)) {
            return None;
        }
        previous = Some(number);
        shown = after;
    }
    shown
        .split_whitespace()
        .eq(rest.split_whitespace())
        .then_some(input)
}

mod paste;
pub(crate) mod post_paste;
#[cfg(test)]
mod tests;
