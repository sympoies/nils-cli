use anyhow::{anyhow, Context, Result};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn is_help(arg: &str) -> bool {
    matches!(arg, "help" | "--help" | "-h")
}

pub fn join_args(args: &[String]) -> String {
    args.join(" ")
}

pub fn env_or_default(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

pub fn env_is_true(name: &str) -> bool {
    match env::var(name) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "y" | "on"
        ),
        Err(_) => false,
    }
}

pub fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn cmd_exists(cmd: &str) -> bool {
    if cmd.contains('/') {
        return Path::new(cmd).is_file();
    }

    let path_var: OsString = match env::var_os("PATH") {
        Some(v) => v,
        None => return false,
    };

    for dir in env::split_paths(&path_var) {
        let full = dir.join(cmd);
        if let Ok(meta) = fs::metadata(&full) {
            if !meta.is_file() {
                continue;
            }
            if meta.permissions().mode() & 0o111 != 0 {
                return true;
            }
        }
    }

    false
}

pub fn run_capture(cmd: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawn {cmd}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "{cmd} failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn run_output(cmd: &str, args: &[&str]) -> Result<Output> {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawn {cmd}"))
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

pub fn shell_escape_single_quotes(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let escaped = value.replace('\'', r#"'\''"#);
    format!("'{escaped}'")
}

pub fn zsh_root() -> Result<PathBuf> {
    if let Ok(v) = env::var("ZDOTDIR") {
        return Ok(PathBuf::from(v));
    }
    let home = env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/zsh"))
}

pub fn zsh_cache_dir() -> Result<PathBuf> {
    if let Ok(v) = env::var("ZSH_CACHE_DIR") {
        return Ok(PathBuf::from(v));
    }
    Ok(zsh_root()?.join("cache"))
}
