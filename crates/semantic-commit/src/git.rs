use std::path::Path;
use std::process::{Command, Stdio};

pub fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub fn is_inside_work_tree(repo: Option<&Path>) -> bool {
    let output = git_command(repo)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim() == "true"
        }
        _ => false,
    }
}

pub fn has_staged_changes(repo: Option<&Path>) -> anyhow::Result<bool> {
    let status = git_command(repo)
        .args(["diff", "--cached", "--quiet", "--"])
        .status()?;

    match status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Ok(!status.success()),
    }
}

fn git_command(repo: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }

    command
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null());

    command
}
