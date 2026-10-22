//! Shared pane input admission and delivery.
use crate::*;

pub(crate) fn send_input_unlocked(
    context: &CliContext,
    record: &SessionRecord,
    text: Option<&str>,
    keys: &[SpecialKey],
    tmux_bin: &Path,
    manual_input: Option<&mut ManualInputSection>,
    paste_mode: PasteMode,
) -> Result<(), CliError> {
    crate::auth_incident::recovery::guard_input(context, record, text, keys)?;
    deliver_input_unlocked(
        context,
        record,
        text,
        keys,
        tmux_bin,
        manual_input,
        paste_mode,
    )
}

/// The sealed command can only be constructed by the restart controller after
/// identity/idle admission and input fencing. External send paths cannot opt
/// out of quarantine, even when their text happens to be `/exit`.
pub(crate) fn send_restart_exit_unlocked(
    context: &CliContext,
    record: &SessionRecord,
    command: crate::restart::ProviderExitCommand,
    tmux_bin: &Path,
) -> Result<(), CliError> {
    deliver_input_unlocked(
        context,
        record,
        Some(command.text()),
        &[SpecialKey::Enter],
        tmux_bin,
        None,
        PasteMode::Raw,
    )
}

fn deliver_input_unlocked(
    context: &CliContext,
    record: &SessionRecord,
    text: Option<&str>,
    keys: &[SpecialKey],
    tmux_bin: &Path,
    mut manual_input: Option<&mut ManualInputSection>,
    paste_mode: PasteMode,
) -> Result<(), CliError> {
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
