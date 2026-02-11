use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use pretty_assertions::assert_eq;
use serde_json::Value;
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

fn stdout(output: &CmdOutput) -> String {
    output.stdout_text()
}

fn assert_exit(output: &CmdOutput, code: i32) {
    assert_eq!(output.code, code);
}

#[test]
fn rate_limits_async_json_one_line_conflict_is_structured() {
    let output = run(
        &["diag", "rate-limits", "--async", "--json", "--one-line"],
        &[],
        &[],
    );
    assert_exit(&output, 64);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.diag.rate-limits.v1");
    assert_eq!(payload["command"], "diag rate-limits");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "invalid-flag-combination");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--async does not support --one-line")
    );
}

#[test]
fn rate_limits_async_json_positional_arg_is_structured() {
    let output = run(
        &["diag", "rate-limits", "--async", "--json", "alpha.json"],
        &[],
        &[],
    );
    assert_exit(&output, 64);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.diag.rate-limits.v1");
    assert_eq!(payload["command"], "diag rate-limits");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "invalid-positional-arg");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--async does not accept positional args")
    );
}

#[test]
fn rate_limits_async_json_cached_clear_cache_conflict_is_structured() {
    let output = run(
        &["diag", "rate-limits", "--async", "--json", "--cached", "-c"],
        &[],
        &[],
    );
    assert_exit(&output, 64);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.diag.rate-limits.v1");
    assert_eq!(payload["command"], "diag rate-limits");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "invalid-flag-combination");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("-c is not compatible with --cached")
    );
}

#[test]
fn rate_limits_async_json_missing_secret_dir_is_structured() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let missing = dir.path().join("missing");

    let output = run(
        &["diag", "rate-limits", "--async", "--format", "json"],
        &[("CODEX_SECRET_DIR", &missing)],
        &[("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false")],
    );
    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.diag.rate-limits.v1");
    assert_eq!(payload["command"], "diag rate-limits");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "secret-discovery-failed");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("CODEX_SECRET_DIR not found")
    );
}

#[test]
fn rate_limits_async_json_outputs_results() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secret_dir = dir.path().join("secrets");
    fs::create_dir_all(&secret_dir).expect("secret dir");
    fs::write(
        secret_dir.join("alpha.json"),
        r#"{"tokens":{"access_token":"tok-alpha","account_id":"acct_001"}}"#,
    )
    .expect("write alpha");
    fs::write(
        secret_dir.join("beta.json"),
        r#"{"tokens":{"access_token":"tok-beta","account_id":"acct_002"}}"#,
    )
    .expect("write beta");

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(
            200,
            r#"{
  "rate_limit": {
    "primary_window": { "limit_window_seconds": 18000, "used_percent": 6, "reset_at": 1700003600 },
    "secondary_window": { "limit_window_seconds": 604800, "used_percent": 12, "reset_at": 1700600000 }
  }
}"#,
        ),
    );

    let output = run(
        &["diag", "rate-limits", "--async", "--json"],
        &[("CODEX_SECRET_DIR", &secret_dir)],
        &[
            ("CODEX_CHATGPT_BASE_URL", &server.url()),
            ("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false"),
            ("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
            ("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3"),
        ],
    );
    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.diag.rate-limits.v1");
    assert_eq!(payload["command"], "diag rate-limits");
    assert_eq!(payload["mode"], "async");
    assert_eq!(payload["ok"], true);
    let results = payload["results"].as_array().expect("results");
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|entry| entry["ok"] == true));
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

#[test]
fn rate_limits_async_clear_cache_failure_reports_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secret_dir = dir.path().join("secrets");
    fs::create_dir_all(&secret_dir).expect("secret dir");

    let output = run(
        &["diag", "rate-limits", "--async", "-c"],
        &[("CODEX_SECRET_DIR", &secret_dir)],
        &[("ZSH_CACHE_DIR", "relative-cache")],
    );
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("refusing to clear cache"));
}
