use std::process::Command;

pub fn is_inside_work_tree() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn has_staged_changes() -> anyhow::Result<bool> {
    let status = Command::new("git")
        .args(["diff", "--cached", "--quiet", "--"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    match status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Ok(!status.success()),
    }
}
