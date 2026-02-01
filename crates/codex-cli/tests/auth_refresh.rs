use std::fs;
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
fn auth_refresh_missing_token() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let auth_file = dir.path().join("auth.json");
    fs::write(&auth_file, r#"{"tokens":{"access_token":"tok"}}"#).expect("write auth");

    let output = run(&["auth", "refresh"], &[("CODEX_AUTH_FILE", &auth_file)]);
    assert_exit(&output, 2);
    assert!(stderr(&output).contains("failed to read refresh token"));
}

#[test]
fn auth_refresh_invalid_name() {
    let output = run(&["auth", "refresh", "../bad.json"], &[]);
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("invalid secret file name"));
}

#[test]
fn auth_refresh_missing_secret_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");

    let output = run(
        &["auth", "refresh", "missing.json"],
        &[("CODEX_SECRET_DIR", &secrets)],
    );
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("not found"));
}
