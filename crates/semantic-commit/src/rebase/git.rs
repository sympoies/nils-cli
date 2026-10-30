use std::{
    path::Path,
    process::{Command, Output, Stdio},
};

pub(super) fn command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

pub(super) fn inspect(root: &Path, args: &[&str]) -> Result<Output, String> {
    let output = command(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .map_err(|_| "rebase_git_probe_unavailable".to_owned())?;
    if output.stdout.len() > 1024 * 1024 || output.stderr.len() > 1024 * 1024 {
        return Err("rebase_git_probe_output_limit".to_owned());
    }
    Ok(output)
}

pub(super) fn read(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = inspect(root, args)?;
    if !output.status.success() {
        return Err("rebase_git_probe_failed".to_owned());
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|_| "rebase_git_probe_unreadable".to_owned())
}
