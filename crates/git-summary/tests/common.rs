use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn git(dir: &Path, args: &[&str]) -> String {
    run_git(dir, args, &[])
}

pub fn git_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    run_git(dir, args, envs)
}

fn run_git(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    for (key, value) in envs {
        cmd.env(key, value);
    }
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

pub fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

pub fn git_summary_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_git-summary")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_git_summary"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("git-summary");
    if bin.exists() {
        return bin;
    }

    panic!("git-summary binary path: NotPresent");
}

pub fn run_git_summary(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(git_summary_bin());
    cmd.args(args).current_dir(dir).stdout(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run git-summary");
    if !output.status.success() {
        panic!(
            "git-summary {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[allow(dead_code)]
pub fn run_git_summary_allow_fail(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> (i32, String) {
    let mut cmd = Command::new(git_summary_bin());
    cmd.args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run git-summary");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (code, stdout)
}
