use super::{CliError, SessionRecord, delete_tmux_buffer, image_path_prompt, run_status};
use std::{fs, path::Path, process::Command};

/// Load the staged file and paste it once. Delete the buffer even if the paste
/// result is uncertain.
pub(super) fn load_and_paste_buffer(
    tmux_bin: &Path,
    buffer_name: &str,
    target: &str,
    file: &Path,
    record: &SessionRecord,
) -> Result<(), CliError> {
    let preserve_lf = record.agent == "claude"
        && fs::read_to_string(file).is_ok_and(|text| {
            text.trim().contains('\n')
                && text
                    .lines()
                    .any(|line| image_path_prompt(line, &record.cwd))
        });

    let mut load = Command::new(tmux_bin);
    load.arg("load-buffer").arg("-b").arg(buffer_name).arg(file);
    run_status(load, "tmux load-buffer")?;

    let mut paste = Command::new(tmux_bin);
    // Keep the bracketed-paste envelope. Claude's image path recognition
    // needs an LF between the path line and the rest of the prompt.
    paste.args(["paste-buffer", "-p"]);
    if preserve_lf {
        paste.arg("-r");
    }
    paste.args(["-b", buffer_name, "-d", "-t", target]);
    if let Err(err) = run_status(paste, "tmux paste-buffer") {
        delete_tmux_buffer(tmux_bin, buffer_name);
        return Err(err);
    }
    Ok(())
}
