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
    paste_prompt_file_observing(tmux_bin, record, prompt_file, pane_ready_deadline, None)
}

fn paste_prompt_file_observing(
    tmux_bin: &Path,
    record: &SessionRecord,
    prompt_file: &Path,
    pane_ready_deadline: Duration,
    before_enter: Option<&mut dyn FnMut(&str)>,
) -> Result<PromptDeliveryObservation, CliError> {
    let buffer_name = format!("{}-prompt", record.id);
    let target = format!("{}:0.0", record.tmux_session);

    // The caller's fixed pre-paste delay is only a guess at provider startup.
    // Confirm the pane has actually been drawn before pasting: bytes delivered
    // while the launch shell still owns the tty are echoed into scrollback and
    // lost, and the single-Enter recovery cannot restore a prompt that never
    // reached the provider's input at all.
    await_pane_drawn_before_paste(tmux_bin, &target, pane_ready_deadline);

    // A drawn pane is still not proof that the provider reads stdin yet: Claude
    // Code parks its cursor on the input line before it accepts input. Ordinary
    // starts may therefore use a pane digest to prove that the first paste was
    // ignored and retry it while no submit key has been sent.
    let before = capture_pane_digest(tmux_bin, &target);
    load_and_paste_buffer(tmux_bin, &buffer_name, &target, prompt_file)
        .map_err(prompt_delivery_failure)?;

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
    if pane_ignored_paste(before, after) {
        load_and_paste_buffer(tmux_bin, &buffer_name, &target, prompt_file)
            .map_err(prompt_delivery_failure)?;
        thread::sleep(POST_PASTE_KEY_SETTLE_DELAY);
    }
    if let Some(observe) = before_enter
        && let Some(pane) = capture_visible_pane(tmux_bin, &target)
    {
        observe(&pane);
    }
    let mut enter = ProcessCommand::new(tmux_bin);
    enter.arg("send-keys").arg("-t").arg(&target).arg("Enter");
    run_status(enter, "tmux send-keys")?;
    Ok(PromptDeliveryObservation {
        schema_version: "agent-session.prompt-delivery-observation.v1",
        composer,
    })
}

/// Paste one structured prompt into a Claude pane and submit it while the
/// session record lock fences the exact runtime, returning the incarnation the
/// text was written to and its observed provider representation. The staged
/// file keeps the prompt out of argv, and the
/// shared start-path delivery keeps the pane-readiness and ignored-paste guards
/// that a freshly launched provider TUI needs.
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
    // Recheck blocked activity under the runtime lock before touching input.
    activity::refuse_if_blocked(
        context,
        &record,
        "answer the dialog with send --key, then resubmit the prompt",
    )?;
    send_guard::require_empty_composer(&record, tmux_bin)?;
    codex_account::ensure_input_allowed(&record)?;
    auto_resume::cancel_for_manual_input_locked(
        context,
        &record.id,
        &Timestamp::now().to_string(),
    )?;

    let nonce = uuid::Uuid::new_v4();
    let staged = session_dir(context, &record.id).join(format!("structured-prompt-{nonce}"));
    write_private_file(&staged, text.as_bytes())?;
    let mut acknowledgement_text = text.to_string();
    let image_path = image_path_prompt(text, &record.cwd);
    let mut observe = |pane: &str| {
        if let Some(label) = composer_image_label(pane) {
            acknowledgement_text = label;
        }
    };
    let delivered = paste_prompt_file_observing(
        tmux_bin,
        &record,
        &staged,
        CLAUDE_STRUCTURED_PROMPT_PANE_READY_DEADLINE,
        image_path.then_some(&mut observe as &mut dyn FnMut(&str)),
    );
    let _ = fs::remove_file(&staged);
    delivered?;

    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, &record)?;
    Ok(ClaudePromptDelivery {
        session_incarnation: expected_launch_id.to_string(),
        acknowledgement_text,
    })
}

/// Transient matching text is never persisted or exposed as receipt metadata.
pub(crate) struct ClaudePromptDelivery {
    pub(crate) session_incarnation: String,
    pub(crate) acknowledgement_text: String,
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

fn composer_image_label(pane: &str) -> Option<String> {
    let input = send_submit::exact_composer_input("claude", pane)?;
    let number = input.strip_prefix("[Image #")?.strip_suffix(']')?;
    (!number.is_empty()
        && number.len() <= 10
        && number.bytes().all(|b| b.is_ascii_digit())
        && number.parse::<u32>().ok().is_some_and(|n| n > 0))
    .then_some(input)
}

#[cfg(test)]
mod tests;
