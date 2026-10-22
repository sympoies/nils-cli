//! Non-Linux inode-selected holder inventory. Darwin may name another hardlink
//! for a mapped vnode, so filtering selected records by pathname is unsafe.
use super::{CliError, probe, refused};
use std::path::Path;
use std::process::Output;

pub(super) fn processes_idle(target: &Path, excluded: &[u32]) -> Result<(), CliError> {
    // +D bounds stdout to target inodes, including cwd, descriptors and mapped
    // text. Keep aliases selected by inode even when their reported name lies
    // outside the target. Cross mounted filesystems (-x f) to cover all target
    // descendants. A traversal/output/time limit must retain the target.
    let output = probe(
        "lsof",
        &[
            "-nP",
            "-F0pcfn",
            "+D",
            target.to_str().ok_or_else(incomplete)?,
            "-x",
            "f",
        ],
        target.parent().ok_or_else(incomplete)?,
    )?;
    check_output(&output, excluded)
}

fn incomplete() -> CliError {
    refused(
        "removal-proof-unavailable",
        "lsof holder inventory could not prove the target idle; target retained",
    )
}

fn check_output(output: &Output, excluded: &[u32]) -> Result<(), CliError> {
    // Positive evidence takes priority over an incomplete inventory diagnostic.
    check(&output.stdout, excluded)?;
    if !output.stderr.is_empty()
        || !matches!(output.status.code(), Some(0 | 1))
        || (output.status.success() && output.stdout.is_empty())
    {
        return Err(incomplete());
    }
    Ok(())
}

fn check(bytes: &[u8], excluded: &[u32]) -> Result<(), CliError> {
    let mut pid = None;
    let mut command = String::new();
    let mut has_file = false;
    if !bytes.is_empty() && !bytes.ends_with(b"\0") && !bytes.ends_with(b"\0\n") {
        return Err(incomplete());
    }
    for field in bytes.split(|b| *b == 0) {
        let field = field.strip_prefix(b"\n").unwrap_or(field);
        let Some((&kind, value)) = field.split_first() else {
            continue;
        };
        match kind {
            b'p' => {
                if pid.is_some() && !has_file {
                    return Err(incomplete());
                }
                has_file = false;
                pid = Some(
                    std::str::from_utf8(value)
                        .ok()
                        .and_then(|s| s.parse::<u32>().ok())
                        .filter(|pid| *pid > 0)
                        .ok_or_else(incomplete)?,
                );
                command.clear();
            }
            b'c' => command = String::from_utf8_lossy(value).into_owned(),
            b'f' if !value.is_empty() => {
                let pid = pid.ok_or_else(incomplete)?;
                has_file = true;
                if !excluded.contains(&pid) {
                    return Err(refused(
                        "removal-process-active",
                        "a process holds a target inode; its reported hardlink path may differ",
                    ).with_details(serde_json::json!({"pid":pid,"command":command,"backend":"lsof","evidence":"target-inode"})));
                }
            }
            b'n' => {}
            _ => return Err(incomplete()),
        }
    }
    if !bytes.is_empty() && !has_file {
        return Err(incomplete());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
