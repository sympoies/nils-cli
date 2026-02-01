use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn codex_cli_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_codex-cli")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_codex_cli"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("codex-cli");
    if bin.exists() {
        return bin;
    }

    panic!("codex-cli binary path: NotPresent");
}

fn run(args: &[&str], envs: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(codex_cli_bin());
    cmd.args(args);
    for (key, path) in envs {
        cmd.env(key, path);
    }
    cmd.output().expect("run codex-cli")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code));
}

#[test]
fn rate_limits_all_missing_secret_dir() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let missing = dir.path().join("missing");

    let output = run(
        &["diag", "rate-limits", "--all"],
        &[("CODEX_SECRET_DIR", &missing)],
    );
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("CODEX_SECRET_DIR not found"));
}

#[test]
fn rate_limits_all_json_conflict() {
    let output = run(&["diag", "rate-limits", "--all", "--json"], &[]);
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--json is not supported with --all"));
}
