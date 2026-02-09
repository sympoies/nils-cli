use anyhow::{Context, Result};
use nils_common::git as common_git;
use nils_common::git::GitContextError;

pub fn require_git() -> Result<(), &'static str> {
    match common_git::require_repo() {
        Ok(()) => Ok(()),
        Err(GitContextError::GitNotFound) => Err("❗ git is required but was not found in PATH."),
        Err(GitContextError::NotRepository) => {
            Err("⚠️ Not a Git repository. Run this command inside a Git project.")
        }
    }
}

pub fn run_git(args: &[String]) -> Result<String> {
    let args_ref: Vec<&str> = args.iter().map(|arg| arg.as_str()).collect();
    let output = common_git::run_output(&args_ref).with_context(|| format!("git {args:?}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {args:?} failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
