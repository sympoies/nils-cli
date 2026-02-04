#![allow(dead_code)]

use anyhow::{anyhow, Context, Result};
use std::process::{Command, Output, Stdio};

pub(crate) fn trim_trailing_newlines(input: &str) -> String {
    input.trim_end_matches(['\n', '\r']).to_string()
}

fn git_command(args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null());
    cmd
}

pub(crate) fn git_output(args: &[&str]) -> Result<Output> {
    let output = git_command(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawn git {:?}", args))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        ));
    }
    Ok(output)
}

pub(crate) fn git_output_optional(args: &[&str]) -> Option<Output> {
    let output = git_command(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output)
}

pub(crate) fn git_status_success(args: &[&str]) -> bool {
    git_command(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn git_status_code(args: &[&str]) -> Option<i32> {
    git_command(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.code().unwrap_or(1))
}

pub(crate) fn git_stdout_trimmed(args: &[&str]) -> Result<String> {
    let output = git_output(args)?;
    Ok(trim_trailing_newlines(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

pub(crate) fn git_stdout_trimmed_optional(args: &[&str]) -> Option<String> {
    let output = git_output_optional(args)?;
    let out = trim_trailing_newlines(&String::from_utf8_lossy(&output.stdout));
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NameStatusEntry {
    pub status_raw: String,
    pub path: String,
    pub old_path: Option<String>,
}

pub(crate) fn parse_name_status_z(bytes: &[u8]) -> Result<Vec<NameStatusEntry>> {
    let parts: Vec<&[u8]> = bytes.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    let mut out: Vec<NameStatusEntry> = Vec::new();
    let mut i = 0;

    while i < parts.len() {
        let status_raw = String::from_utf8_lossy(parts[i]).to_string();
        i += 1;

        if status_raw.starts_with('R') || status_raw.starts_with('C') {
            let old = parts
                .get(i)
                .ok_or_else(|| anyhow!("error: malformed name-status output"))?;
            let new = parts
                .get(i + 1)
                .ok_or_else(|| anyhow!("error: malformed name-status output"))?;
            i += 2;
            out.push(NameStatusEntry {
                status_raw,
                path: String::from_utf8_lossy(new).to_string(),
                old_path: Some(String::from_utf8_lossy(old).to_string()),
            });
        } else {
            let file = parts
                .get(i)
                .ok_or_else(|| anyhow!("error: malformed name-status output"))?;
            i += 1;
            out.push(NameStatusEntry {
                status_raw,
                path: String::from_utf8_lossy(file).to_string(),
                old_path: None,
            });
        }
    }

    Ok(out)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DiffNumstat {
    pub added: Option<i64>,
    pub deleted: Option<i64>,
    pub binary: bool,
}

pub(crate) fn diff_numstat(path: &str) -> Result<DiffNumstat> {
    let output = git_stdout_trimmed(&[
        "-c",
        "core.quotepath=false",
        "diff",
        "--cached",
        "--numstat",
        "--",
        path,
    ])?;

    let line = output.lines().next().unwrap_or("");
    if line.trim().is_empty() {
        return Ok(DiffNumstat {
            added: None,
            deleted: None,
            binary: false,
        });
    }

    let mut parts = line.split('\t');
    let added = parts.next().unwrap_or("");
    let deleted = parts.next().unwrap_or("");

    if added == "-" || deleted == "-" {
        return Ok(DiffNumstat {
            added: None,
            deleted: None,
            binary: true,
        });
    }

    let added_num = added.parse::<i64>().ok();
    let deleted_num = deleted.parse::<i64>().ok();

    Ok(DiffNumstat {
        added: added_num,
        deleted: deleted_num,
        binary: false,
    })
}

pub(crate) fn is_lockfile(path: &str) -> bool {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    matches!(
        name,
        "yarn.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "bun.lockb"
            | "bun.lock"
            | "npm-shrinkwrap.json"
    )
}
