use anyhow::{Context, Result};
use nils_common::git as common_git;
use std::process::Output;

const DEFAULT_GIT_CONFIG: [&str; 2] = ["-c", "core.quotepath=false"];

pub(crate) fn run_git(args: &[&str]) -> Result<String> {
    let args = with_default_config(args);
    let output = run_git_output(&args)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!("git {args:?} failed: {stderr}{stdout}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn with_default_config<'a>(args: &'a [&'a str]) -> Vec<&'a str> {
    if has_quotepath_override(args) {
        return args.to_vec();
    }

    let mut updated = Vec::with_capacity(args.len() + DEFAULT_GIT_CONFIG.len());
    updated.extend_from_slice(&DEFAULT_GIT_CONFIG);
    updated.extend_from_slice(args);
    updated
}

fn has_quotepath_override(args: &[&str]) -> bool {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if *arg == "-c" {
            if let Some(value) = iter.next() {
                if value.starts_with("core.quotepath=") {
                    return true;
                }
            }
        } else if let Some(rest) = arg.strip_prefix("-c") {
            if rest.starts_with("core.quotepath=") {
                return true;
            }
        }
    }
    false
}

fn run_git_output(args: &[&str]) -> Result<Output> {
    common_git::run_output(args).with_context(|| format!("failed to run git {args:?}"))
}
