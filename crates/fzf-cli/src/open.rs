use crate::util;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenWith {
    Vi,
    Vscode,
}

pub fn parse_open_with_flags(args: &[String]) -> Result<(OpenWith, Vec<String>), i32> {
    let env_default = util::env_or_default("FZF_FILE_OPEN_WITH", "vi");
    let mut open_with = if env_default.trim() == "vscode" {
        OpenWith::Vscode
    } else {
        OpenWith::Vi
    };

    let mut seen_vi = false;
    let mut seen_vscode = false;

    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--vi" => {
                seen_vi = true;
                open_with = OpenWith::Vi;
                i += 1;
            }
            "--vscode" => {
                seen_vscode = true;
                open_with = OpenWith::Vscode;
                i += 1;
            }
            "--" => {
                i += 1;
                rest.extend_from_slice(&args[i..]);
                break;
            }
            flag if flag.starts_with("--") => {
                eprintln!("❌ Unknown flag: {flag}");
                return Err(2);
            }
            _ => {
                rest.extend_from_slice(&args[i..]);
                break;
            }
        }
    }

    if seen_vi && seen_vscode {
        eprintln!("❌ Flags are mutually exclusive: --vi and --vscode");
        return Err(2);
    }

    Ok((open_with, rest))
}

pub fn find_git_root_upwards(start_dir: &Path, max_depth: usize) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    for _ in 0..=max_depth {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn open_in_vscode_workspace(workspace_root: &Path, file: &Path, wait: bool) -> anyhow::Result<()> {
    if !util::cmd_exists("code") {
        eprintln!("❌ 'code' not found");
        anyhow::bail!("code not found");
    }

    let mut args: Vec<String> = Vec::new();
    if wait {
        args.push("--wait".to_string());
    }
    args.push("--goto".to_string());
    args.push(file.to_string_lossy().to_string());
    args.push("--".to_string());
    args.push(workspace_root.to_string_lossy().to_string());

    let status = Command::new("code")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if !status.success() {
        anyhow::bail!("code failed");
    }

    Ok(())
}

pub fn open_in_vscode(file: &Path, wait: bool) -> anyhow::Result<()> {
    if let Some(parent) = file.parent() {
        if let Some(git_root) = find_git_root_upwards(parent, 5) {
            return open_in_vscode_workspace(&git_root, file, wait);
        }
    }

    if !util::cmd_exists("code") {
        eprintln!("❌ 'code' not found");
        anyhow::bail!("code not found");
    }

    let mut args: Vec<String> = Vec::new();
    if wait {
        args.push("--wait".to_string());
    }
    args.push("--goto".to_string());
    args.push(file.to_string_lossy().to_string());

    let status = Command::new("code")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if !status.success() {
        anyhow::bail!("code failed");
    }

    Ok(())
}

pub fn open_file(open_with: OpenWith, file: &Path, wait: bool) -> i32 {
    match open_with {
        OpenWith::Vscode => {
            if open_in_vscode(file, wait).is_err() {
                eprintln!("❌ Failed to open in VSCode; falling back to vi");
                return open_vi(file);
            }
            0
        }
        OpenWith::Vi => open_vi(file),
    }
}

pub fn open_file_in_workspace(
    open_with: OpenWith,
    workspace_root: &Path,
    file: &Path,
    wait: bool,
) -> i32 {
    match open_with {
        OpenWith::Vscode => {
            if open_in_vscode_workspace(workspace_root, file, wait).is_err() {
                eprintln!("❌ Failed to open in VSCode; falling back to vi");
                return open_vi(file);
            }
            0
        }
        OpenWith::Vi => open_vi(file),
    }
}

fn open_vi(file: &Path) -> i32 {
    let status = Command::new("vi")
        .arg("--")
        .arg(file)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 127,
    }
}
