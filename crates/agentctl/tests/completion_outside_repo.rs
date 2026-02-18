use nils_test_support::bin;
use nils_test_support::cmd;
use std::path::PathBuf;

fn agentctl_bin() -> PathBuf {
    bin::resolve("agentctl")
}

#[test]
fn completion_export_succeeds_outside_git_repo() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let output = cmd::run_in_dir(
        dir.path(),
        &agentctl_bin(),
        &["completion", "zsh"],
        &[],
        None,
    );

    assert_eq!(output.code, 0);
    let stdout = output.stdout_text();
    assert!(stdout.contains("#compdef agentctl"), "stdout={stdout}");
}

#[test]
fn completion_rejects_unknown_shell_outside_git_repo() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let output = cmd::run_in_dir(
        dir.path(),
        &agentctl_bin(),
        &["completion", "fish"],
        &[],
        None,
    );

    assert_ne!(output.code, 0);
    let stderr = output.stderr_text();
    assert!(stderr.contains("invalid value"), "stderr={stderr}");
    assert!(stderr.contains("fish"), "stderr={stderr}");
}
