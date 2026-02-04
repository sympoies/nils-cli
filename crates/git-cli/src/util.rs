use anyhow::{anyhow, Context, Result};
use std::env;
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub fn cmd_exists(cmd: &str) -> bool {
    find_in_path(cmd).is_some()
}

pub fn find_in_path(cmd: &str) -> Option<PathBuf> {
    if cmd.contains('/') {
        let path = Path::new(cmd);
        return is_executable_file(path).then(|| path.to_path_buf());
    }

    let path_var: OsString = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let full = dir.join(cmd);
        if is_executable_file(&full) {
            return Some(full);
        }
    }

    None
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn run_output(cmd: &str, args: &[&str]) -> Result<Output> {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawn {cmd}"))
}

pub fn run_capture(cmd: &str, args: &[&str]) -> Result<String> {
    let output = run_output(cmd, args)?;
    if !output.status.success() {
        return Err(anyhow!(
            "{cmd} failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
