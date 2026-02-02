use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};

fn codex_cli_bin() -> PathBuf {
    bin::resolve("codex-cli")
}

fn run(args: &[&str], envs: &[(&str, &Path)]) -> CmdOutput {
    let mut options = CmdOptions::default();
    for (key, path) in envs {
        let value = path.to_string_lossy();
        options = options.with_env(key, value.as_ref());
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
