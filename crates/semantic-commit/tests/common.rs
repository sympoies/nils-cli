#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub fn git_output(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed to spawn")
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_output(dir, args);
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
    dir
}

pub fn write_file(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    fs::write(path, contents).expect("write file");
}

pub fn semantic_commit_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_semantic-commit")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_semantic_commit"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("semantic-commit");
    if bin.exists() {
        return bin;
    }

    panic!("semantic-commit binary path: NotPresent");
}

pub fn run_semantic_commit_output(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    input: Option<&str>,
) -> Output {
    let mut cmd = Command::new(semantic_commit_bin());
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in envs {
        cmd.env(key, value);
    }

    let mut child = cmd.spawn().expect("spawn semantic-commit");
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(data) = input {
            stdin.write_all(data.as_bytes()).expect("write stdin");
        }
    }

    child.wait_with_output().expect("wait semantic-commit")
}

pub fn write_executable(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    fs::write(&path, contents).expect("write executable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set perms");
    }
}
