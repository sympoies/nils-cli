use std::path::PathBuf;
use std::process::{Command, Stdio};

fn memo_cli_bin() -> PathBuf {
    for env_name in ["CARGO_BIN_EXE_memo-cli", "CARGO_BIN_EXE_memo_cli"] {
        if let Some(path) = std::env::var_os(env_name) {
            return PathBuf::from(path);
        }
    }

    let current = std::env::current_exe().expect("current test executable");
    let target_profile_dir = current
        .parent()
        .and_then(|path| path.parent())
        .expect("target profile dir");
    let candidate = target_profile_dir.join(format!("memo-cli{}", std::env::consts::EXE_SUFFIX));
    assert!(
        candidate.exists(),
        "memo-cli binary path not found via env vars or fallback candidate {}",
        candidate.display()
    );
    candidate
}

#[test]
fn completion_export_succeeds_outside_git_repo() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(memo_cli_bin())
        .args(["completion", "zsh"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run memo-cli completion zsh");

    assert!(
        output.status.success(),
        "expected exit code 0, got: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("#compdef memo-cli"),
        "missing zsh completion header: {stdout}"
    );
}

#[test]
fn completion_rejects_unknown_shell_outside_git_repo() {
    let temp = tempfile::TempDir::new().unwrap();
    let output = Command::new(memo_cli_bin())
        .args(["completion", "fish"])
        .current_dir(temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run memo-cli completion fish");

    assert!(
        !output.status.success(),
        "expected non-zero exit code for unknown shell, got: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value") && stderr.contains("fish"),
        "missing invalid shell error: {stderr}"
    );
}
