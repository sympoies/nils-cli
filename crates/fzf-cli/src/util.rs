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

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn env_is_true_accepts_known_truthy_values() {
        let lock = GlobalStateLock::new();
        for value in ["1", " true ", "YES", "y", "On"] {
            let _guard = EnvGuard::set(&lock, "FZF_CLI_TEST_BOOL", value);
            assert!(
                env_is_true("FZF_CLI_TEST_BOOL"),
                "expected truthy value: {value}"
            );
        }
    }

    #[test]
    fn env_is_true_rejects_missing_and_unknown_values() {
        let lock = GlobalStateLock::new();
        let _unset = EnvGuard::remove(&lock, "FZF_CLI_TEST_BOOL");
        assert!(!env_is_true("FZF_CLI_TEST_BOOL"));

        for value in ["", "0", "false", "no", "off", "2", " maybe "] {
            let _guard = EnvGuard::set(&lock, "FZF_CLI_TEST_BOOL", value);
            assert!(
                !env_is_true("FZF_CLI_TEST_BOOL"),
                "expected falsey value: {value}"
            );
        }
    }

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        let input = "\x1b[31mred\x1b[0m plain \x1b[38;5;110mblue\x1b[0m";
        assert_eq!(strip_ansi(input), "red plain blue");
    }

    #[test]
    fn shell_escape_single_quotes_matches_current_behavior() {
        assert_eq!(shell_escape_single_quotes(""), "''");
        assert_eq!(shell_escape_single_quotes("plain"), "'plain'");
        assert_eq!(shell_escape_single_quotes("a'b"), "'a'\\''b'");
        assert_eq!(
            shell_escape_single_quotes("'start and end'"),
            "''\\''start and end'\\'''"
        );
    }

    #[test]
    fn git_repo_probe_semantics_success_and_failure_are_stable() {
        fn probe() -> bool {
            run_output("git", &["rev-parse", "--is-inside-work-tree"])
                .map(|output| output.status.success())
                .unwrap_or(false)
        }

        let lock = GlobalStateLock::new();

        let success_stubs = StubBinDir::new();
        success_stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == "rev-parse" && "${2:-}" == "--is-inside-work-tree" ]]; then
  exit 0
fi
exit 1
"#,
        );
        let _path_success = EnvGuard::set(&lock, "PATH", &success_stubs.path_str());
        assert!(probe());
        drop(_path_success);

        let fail_stubs = StubBinDir::new();
        fail_stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == "rev-parse" && "${2:-}" == "--is-inside-work-tree" ]]; then
  exit 128
fi
exit 1
"#,
        );
        let _path_fail = EnvGuard::set(&lock, "PATH", &fail_stubs.path_str());
        assert!(!probe());
        drop(_path_fail);

        let empty = TempDir::new().expect("tempdir");
        let empty_path = empty.path().to_string_lossy().to_string();
        let _path_missing = EnvGuard::set(&lock, "PATH", &empty_path);
        assert!(!probe());
    }
}
