use anyhow::{Context, Result};
use nils_common::git as common_git;

pub fn require_git() -> Result<(), &'static str> {
    if common_git::run_status_quiet(&["--version"]).is_err() {
        return Err("❗ git is required but was not found in PATH.");
    }

    let status = common_git::run_status_quiet(&["rev-parse", "--git-dir"]);

    if !status.map(|s| s.success()).unwrap_or(false) {
        return Err("⚠️ Not a Git repository. Run this command inside a Git project.");
    }

    Ok(())
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
