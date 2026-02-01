#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct CmdOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn plan_tooling_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_plan-tooling")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_plan_tooling"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("plan-tooling");
    if bin.exists() {
        return bin;
    }

    panic!("plan-tooling binary path: NotPresent");
}

pub fn run_plan_tooling(dir: &Path, args: &[&str]) -> CmdOut {
    let mut cmd = Command::new(plan_tooling_bin());
    cmd.current_dir(dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = cmd.output().expect("run plan-tooling");
    CmdOut {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

pub fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create_dir_all");
    }
    std::fs::write(path, contents).expect("write");
}

pub fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    let output = cmd.output().expect("git command failed to spawn");

    if !output.status.success() {
        panic!(
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }

    String::from_utf8_lossy(&output.stdout).to_string()
}
