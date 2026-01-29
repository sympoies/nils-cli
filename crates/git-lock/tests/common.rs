#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed to spawn");

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

pub fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["checkout", "-q", "-B", "main"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    git(dir.path(), &["config", "tag.gpgSign", "false"]);

    let file_path = dir.path().join("README.md");
    fs::write(&file_path, "init").expect("write README");
    git(dir.path(), &["add", "README.md"]);
    git(dir.path(), &["commit", "-m", "init"]);

    dir
}

pub fn commit_file(dir: &Path, name: &str, contents: &str, message: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write file");
    git(dir, &["add", name]);
    git(dir, &["commit", "-m", message]);
    git(dir, &["rev-parse", "HEAD"]).trim().to_string()
}

pub fn repo_id(dir: &Path) -> String {
    dir.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string()
}

pub fn git_lock_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_git-lock").or_else(|_| std::env::var("CARGO_BIN_EXE_git_lock"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("git-lock");
    if bin.exists() {
        return bin;
    }

    panic!("git-lock binary path: NotPresent");
}

pub fn run_git_lock_output(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> Output {
    let mut cmd = Command::new(git_lock_bin());
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in envs {
        cmd.env(key, value);
    }

    let mut child = cmd.spawn().expect("spawn git-lock");
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(data) = input {
            stdin.write_all(data.as_bytes()).expect("write stdin");
        }
    }

    child.wait_with_output().expect("wait git-lock")
}

pub fn run_git_lock(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> String {
    let output = run_git_lock_output(dir, args, envs, input);
    if !output.status.success() {
        panic!(
            "git-lock {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}
