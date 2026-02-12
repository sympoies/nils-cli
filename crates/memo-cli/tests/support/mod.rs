#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct CmdOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CmdOutput {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

pub fn test_db_path(name: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    dir.keep().join(format!("{name}.db"))
}

pub fn parse_json_stdout(output: &CmdOutput) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

pub fn fixture_json(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    let raw = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    });
    serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!("failed to parse fixture {}: {err}", path.display());
    })
}

pub fn run_memo_cli(db_path: &Path, args: &[&str], stdin: Option<&str>) -> CmdOutput {
    run_memo_cli_with_env(db_path, args, stdin, &[])
}

pub fn run_memo_cli_with_env(
    db_path: &Path,
    args: &[&str],
    stdin: Option<&str>,
    envs: &[(&str, &str)],
) -> CmdOutput {
    let db = db_path.display().to_string();
    let mut argv = vec!["--db", db.as_str()];
    argv.extend_from_slice(args);

    let mut cmd = Command::new(memo_cli_bin());
    cmd.args(&argv)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }

    let output = match stdin {
        Some(input) => {
            cmd.stdin(Stdio::piped());
            let mut child = cmd.spawn().expect("spawn memo-cli");
            if let Some(mut writer) = child.stdin.take() {
                writer
                    .write_all(input.as_bytes())
                    .expect("write stdin to memo-cli");
            }
            child.wait_with_output().expect("wait memo-cli output")
        }
        None => {
            cmd.stdin(Stdio::null());
            cmd.output().expect("run memo-cli")
        }
    };

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn memo_cli_bin() -> PathBuf {
    for env_name in ["CARGO_BIN_EXE_memo-cli", "CARGO_BIN_EXE_memo_cli"] {
        if let Ok(path) = std::env::var(env_name) {
            return PathBuf::from(path);
        }
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe
        .parent()
        .and_then(|path| path.parent())
        .expect("target dir");
    let fallback = target_dir.join(format!("memo-cli{}", std::env::consts::EXE_SUFFIX));
    if fallback.exists() {
        return fallback;
    }

    panic!("memo-cli binary path not found via CARGO_BIN_EXE_* or target fallback");
}
