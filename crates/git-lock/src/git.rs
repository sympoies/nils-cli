use anyhow::{Context, Result};
use std::process::{Command, Stdio};

pub trait GitBackend {
    fn log_subject(&self, hash: &str) -> Result<Option<String>>;
}

#[derive(Debug, Default)]
pub struct DefaultGitBackend;

impl GitBackend for DefaultGitBackend {
    fn log_subject(&self, hash: &str) -> Result<Option<String>> {
        log_subject(hash)
    }
}

pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn run_capture(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("git {args:?}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {args:?} failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn run_capture_optional(args: &[&str]) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("git {args:?}"))?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

pub fn run_status_inherit(args: &[&str]) -> Result<i32> {
    let status = Command::new("git")
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("git {args:?}"))?;

    Ok(status.code().unwrap_or(1))
}

pub fn rev_parse(value: &str) -> Result<Option<String>> {
    run_capture_optional(&["rev-parse", value])
}

pub fn show_subject(hash: &str) -> Result<Option<String>> {
    run_capture_optional(&["show", "-s", "--format=%s", hash])
}

pub fn log_subject(hash: &str) -> Result<Option<String>> {
    run_capture_optional(&["log", "-1", "--pretty=format:%s", hash])
}

pub fn tag_exists(tag: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["rev-parse", tag])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("git rev-parse {tag}"))?;

    Ok(output.success())
}
