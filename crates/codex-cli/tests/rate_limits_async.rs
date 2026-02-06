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
fn rate_limits_async_json_conflict() {
    let output = run(&["diag", "rate-limits", "--async", "--json"], &[], &[]);
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--async does not support --json"));
}

#[test]
fn rate_limits_async_one_line_conflict() {
    let output = run(&["diag", "rate-limits", "--async", "--one-line"], &[], &[]);
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--async does not support --one-line"));
}

#[test]
fn rate_limits_async_jobs_zero_defaults() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secret_dir = dir.path().join("secrets");
    fs::create_dir_all(&secret_dir).expect("secret dir");

    let output = run(
        &["diag", "rate-limits", "--async", "--jobs", "0"],
        &[("CODEX_SECRET_DIR", &secret_dir)],
        &[],
    );
    assert_exit(&output, 1);
    let err = stderr(&output);
    assert!(err.contains("no secrets found"));
    assert!(!err.contains("invalid --jobs value"));
}

#[test]
fn rate_limits_async_missing_secret_dir() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let missing = dir.path().join("missing");

    let output = run(
        &["diag", "rate-limits", "--async"],
        &[("CODEX_SECRET_DIR", &missing)],
        &[],
    );
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("CODEX_SECRET_DIR not found"));
}

#[test]
fn rate_limits_async_rejects_positional_secret_arg() {
    let output = run(&["diag", "rate-limits", "--async", "alpha.json"], &[], &[]);
    assert_exit(&output, 64);
    let err = stderr(&output);
    assert!(err.contains("--async does not accept positional args: alpha.json"));
    assert!(err.contains("hint: async always queries all secrets under CODEX_SECRET_DIR"));
}

#[test]
fn rate_limits_async_rejects_cached_clear_cache_combo() {
    let output = run(
        &["diag", "rate-limits", "--async", "--cached", "-c"],
        &[],
        &[],
    );
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--async: -c is not compatible with --cached"));
}
