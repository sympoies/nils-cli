use std::path::PathBuf;
use std::process::{Command, Stdio};

fn git_scope_bin() -> PathBuf {
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

#[test]
fn help_flag_succeeds_outside_git_repo() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(git_scope_bin())
        .args(["--help"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run git-scope --help");

    assert!(
        output.status.success(),
        "expected exit code 0, got: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: git-scope"),
        "missing Usage: {stdout}"
    );
    assert!(
        !stdout.contains("Not a Git repository"),
        "unexpected repo warning: {stdout}"
    );
}

#[test]
fn help_subcommand_succeeds_outside_git_repo() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(git_scope_bin())
        .args(["help"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run git-scope help");

    assert!(
        output.status.success(),
        "expected exit code 0, got: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: git-scope"),
        "missing Usage: {stdout}"
    );
    assert!(
        !stdout.contains("Not a Git repository"),
        "unexpected repo warning: {stdout}"
    );
}

#[test]
fn version_flag_succeeds_outside_git_repo() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(git_scope_bin())
        .args(["--version"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run git-scope --version");

    assert!(
        output.status.success(),
        "expected exit code 0, got: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("git-scope"),
        "missing binary name: {stdout}"
    );
    assert!(
        !stdout.contains("Not a Git repository"),
        "unexpected repo warning: {stdout}"
    );
}

#[test]
fn root_help_does_not_advertise_subcommand_print_flag() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(git_scope_bin())
        .args(["--help"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run git-scope --help");

    assert!(
        output.status.success(),
        "expected exit code 0, got: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("-p, --print"),
        "root help should not show subcommand-only flags: {stdout}"
    );
}

#[test]
fn subcommand_help_uses_subcommand_scope() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(git_scope_bin())
        .args(["all", "--help"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run git-scope all --help");

    assert!(
        output.status.success(),
        "expected exit code 0, got: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: all [OPTIONS]"),
        "expected subcommand usage in help output: {stdout}"
    );
    assert!(
        stdout.contains("-p, --print"),
        "expected subcommand print option in help output: {stdout}"
    );
}

#[test]
fn completion_export_succeeds_outside_git_repo() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(git_scope_bin())
        .args(["completion", "zsh"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run git-scope completion zsh");

    assert!(
        output.status.success(),
        "expected exit code 0, got: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("#compdef git-scope"),
        "missing zsh completion header: {stdout}"
    );
    assert!(
        !stdout.contains("Not a Git repository"),
        "unexpected repo warning: {stdout}"
    );
}

#[test]
fn completion_rejects_unknown_shell_outside_git_repo() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(git_scope_bin())
        .args(["completion", "fish"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run git-scope completion fish");

    assert!(
        !output.status.success(),
        "expected non-zero exit code for unknown shell, got: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Not a Git repository"),
        "unexpected repo warning: {stderr}"
    );
}
