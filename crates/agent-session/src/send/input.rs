//! Shared pane input admission and delivery.
use crate::*;

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
