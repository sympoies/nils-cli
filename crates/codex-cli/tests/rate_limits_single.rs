use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::{Path, PathBuf};

fn codex_cli_bin() -> PathBuf {
    bin::resolve("codex-cli")
}

fn run(args: &[&str], envs: &[(&str, &Path)], vars: &[(&str, &str)]) -> CmdOutput {
    let mut options = CmdOptions::default();
    for (key, path) in envs {
        let value = path.to_string_lossy();
        options = options.with_env(key, value.as_ref());
    }
    for (key, value) in vars {
        options = options.with_env(key, value);
    }
    let bin = codex_cli_bin();
    cmd::run_with(&bin, args, &options)
}

fn stderr(output: &CmdOutput) -> String {
    output.stderr_text()
}

fn assert_exit(output: &CmdOutput, code: i32) {
    assert_eq!(output.code, code);
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

#[test]
fn rate_limits_single_cached_json_conflict() {
    let output = run(
        &["diag", "rate-limits", "--cached", "--json"],
        &[],
        &[("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false")],
    );
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--json is not supported with --cached"));
}

#[test]
fn rate_limits_single_cached_clear_cache_conflict() {
    let output = run(
        &["diag", "rate-limits", "--cached", "-c"],
        &[],
        &[("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false")],
    );
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("-c is not compatible with --cached"));
}
