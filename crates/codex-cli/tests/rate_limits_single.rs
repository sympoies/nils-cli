use pretty_assertions::assert_eq;
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

fn run(args: &[&str], envs: &[(&str, &Path)], vars: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(codex_cli_bin());
    cmd.args(args);
    for (key, path) in envs {
        cmd.env(key, path);
    }
    for (key, value) in vars {
        cmd.env(key, value);
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
fn rate_limits_single_json_one_line_conflict() {
    let output = run(
        &["diag", "rate-limits", "--json", "--one-line"],
        &[],
        &[("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false")],
    );
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--one-line is not compatible with --json"));
}

#[test]
fn rate_limits_single_cached_missing_cache() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let auth_file = dir.path().join("auth.json");
    fs::write(&auth_file, r#"{"tokens":{"access_token":"tok"}}"#).expect("write auth");

    let cache_dir = dir.path().join("cache");
    fs::create_dir_all(&cache_dir).expect("cache dir");

    let output = run(
        &["diag", "rate-limits", "--cached"],
        &[
            ("CODEX_AUTH_FILE", &auth_file),
            ("CODEX_SECRET_CACHE_DIR", &cache_dir),
        ],
        &[("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false")],
    );
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("cache not found"));
}
