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
    // Make the initial branch deterministic across environments (some git configs default to
    // `master`, others to `main`).
    git(dir.path(), &["checkout", "-q", "-B", "main"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

pub fn git_scope_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_git-scope")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_git_scope"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("git-scope");
    if bin.exists() {
        return bin;
    }

    panic!("git-scope binary path: NotPresent");
}

pub fn run_git_scope(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let output = run_git_scope_output(dir, args, envs);
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn run_git_scope_output(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(git_scope_bin());
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run git-scope");
    if !output.status.success() {
        panic!(
            "git-scope {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    output
}
