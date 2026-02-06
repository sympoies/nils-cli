use anyhow::{Context, Result};
use nils_common::git as common_git;

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
    common_git::is_git_repo().unwrap_or(false)
}

pub fn run_capture(args: &[&str]) -> Result<String> {
    let output = run_git_output(args)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {args:?} failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn run_capture_optional(args: &[&str]) -> Result<Option<String>> {
    let output = run_git_output(args)?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

pub fn run_status_inherit(args: &[&str]) -> Result<i32> {
    let status = common_git::run_status_inherit(args).with_context(|| format!("git {args:?}"))?;

    Ok(status.code().unwrap_or(1))
}

pub fn rev_parse(value: &str) -> Result<Option<String>> {
    common_git::rev_parse(&[value]).with_context(|| format!("git rev-parse {value}"))
}

pub fn show_subject(hash: &str) -> Result<Option<String>> {
    run_capture_optional(&["show", "-s", "--format=%s", hash])
}

pub fn log_subject(hash: &str) -> Result<Option<String>> {
    run_capture_optional(&["log", "-1", "--pretty=format:%s", hash])
}

pub fn tag_exists(tag: &str) -> Result<bool> {
    let output = common_git::run_status_quiet(&["rev-parse", tag])
        .with_context(|| format!("git rev-parse {tag}"))?;

    Ok(output.success())
}

fn run_git_output(args: &[&str]) -> Result<std::process::Output> {
    common_git::run_output(args).with_context(|| format!("git {args:?}"))
}
