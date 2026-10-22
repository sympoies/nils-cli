//! Shared pane input admission and delivery.
use crate::*;

/// Paste into the exact locked runtime using the existing provider readiness
/// and ignored-paste checks. The prompt remains in a private staged file.
pub(crate) fn deliver_claude_structured_prompt_locked(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    text: &str,
    tmux_bin: &Path,
) -> Result<String, CliError> {
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
    let delivered = paste_prompt_file(
        tmux_bin,
        &record,
        &staged,
        CLAUDE_STRUCTURED_PROMPT_PANE_READY_DEADLINE,
    );
    let _ = fs::remove_file(&staged);
    delivered?;

    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, &record)?;
    Ok(expected_launch_id.to_string())
}

pub(crate) fn send_input_unlocked(
    context: &CliContext,
    record: &SessionRecord,
    text: Option<&str>,
    keys: &[SpecialKey],
    tmux_bin: &Path,
    mut manual_input: Option<&mut ManualInputSection>,
    paste_mode: PasteMode,
) -> Result<(), CliError> {
    crate::auth_incident::recovery::guard_input(context, record, text, keys)?;
    let target = format!("{}:0.0", record.tmux_session);
    let mut pasted_literal_text = false;
    if let Some(text) = text {
        if text_is_bare_newline(text) {
            if let Some(section) = manual_input.as_deref_mut() {
                section.arm(context, record)?;
            }
            send_tmux_key(tmux_bin, &target, SpecialKey::Enter)?;
        } else {
            let nonce = uuid::Uuid::new_v4();
            let buffer_name = format!("{}-send-{nonce}", record.id);
            let temp = session_dir(context, &record.id).join(format!("send-input-{nonce}"));
            write_private_file(&temp, text.as_bytes())?;
            let result = load_and_paste_buffer_with_timeout(
                tmux_bin,
                &buffer_name,
                &target,
                &temp,
                PANE_INPUT_COMMAND_TIMEOUT,
                paste_mode,
            );
            let _ = fs::remove_file(&temp);
            result?;
            pasted_literal_text = true;
        }
    }
    if let Some(delay) = post_paste_settle_delay(pasted_literal_text, !keys.is_empty()) {
        thread::sleep(delay);
    }
    for key in keys {
        if *key == SpecialKey::Enter
            && let Some(section) = manual_input.as_deref_mut()
        {
            section.arm(context, record)?;
        }
        send_tmux_key(tmux_bin, &target, *key)?;
    }
    Ok(())
}
