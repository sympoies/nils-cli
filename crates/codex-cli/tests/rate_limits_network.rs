use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn stdout(output: &CmdOutput) -> String {
    output.stdout_text()
}

fn stderr(output: &CmdOutput) -> String {
    output.stderr_text()
}

fn assert_exit(output: &CmdOutput, code: i32) {
    assert_eq!(
        output.code,
        code,
        "unexpected exit code.\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

fn write_secret(dir: &Path, name: &str, access_token: Option<&str>) -> PathBuf {
    let path = dir.join(name);
    let json = match access_token {
        Some(token) => format!(
            r#"{{
  "tokens": {{
    "access_token": "{token}",
    "account_id": "acct_001"
  }}
}}"#
        ),
        None => r#"{"tokens":{"account_id":"acct_001"}}"#.to_string(),
    };
    fs::write(&path, json).expect("write secret");
    path
}

fn wham_usage_ok_body() -> String {
    r#"{
  "rate_limit": {
    "primary_window": { "limit_window_seconds": 18000, "used_percent": 6, "reset_at": 1700003600 },
    "secondary_window": { "limit_window_seconds": 604800, "used_percent": 12, "reset_at": 1700600000 }
  }
}"#
    .to_string()
}

fn cache_kv_path(cache_root: &Path, key: &str) -> PathBuf {
    cache_root
        .join("codex")
        .join("starship-rate-limits")
        .join(format!("{key}.kv"))
}

#[test]
fn rate_limits_single_default_output_from_network() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");
    write_secret(&secrets, "alpha.json", Some("tok"));

    let cache_root = dir.path().join("cache_root");
    fs::create_dir_all(&cache_root).expect("cache root");

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, wham_usage_ok_body()),
    );

    let output = run(
        &["diag", "rate-limits", "alpha.json"],
        &[
            ("CODEX_SECRET_DIR", &secrets),
            ("ZSH_CACHE_DIR", &cache_root),
        ],
        &[
            ("CODEX_CHATGPT_BASE_URL", &server.url()),
            ("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false"),
            ("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
            ("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3"),
            ("TZ", "UTC"),
            ("NO_COLOR", "1"),
        ],
    );
    assert_exit(&output, 0);
    assert_eq!(
        stdout(&output),
        "Rate limits remaining\n5h 94% • 11-14 23:13\nWeekly 88% • 11-21 20:53\n"
    );
}

#[test]
fn rate_limits_single_one_line_writes_cache_and_metadata() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");
    let secret_path = write_secret(&secrets, "alpha.json", Some("tok"));

    let cache_root = dir.path().join("cache_root");
    fs::create_dir_all(&cache_root).expect("cache root");

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, wham_usage_ok_body()),
    );

    let output = run(
        &["diag", "rate-limits", "--one-line", "alpha.json"],
        &[
            ("CODEX_SECRET_DIR", &secrets),
            ("ZSH_CACHE_DIR", &cache_root),
        ],
        &[
            ("CODEX_CHATGPT_BASE_URL", &server.url()),
            ("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false"),
            ("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
            ("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3"),
            ("TZ", "UTC"),
            ("NO_COLOR", "1"),
        ],
    );
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "alpha 5h:94% W:88% 11-21 20:53\n");

    let secret_json: Value =
        serde_json::from_str(&fs::read_to_string(&secret_path).expect("read secret"))
            .expect("json");
    assert_eq!(
        secret_json["codex_rate_limits"]["weekly_reset_at_epoch"].as_i64(),
        Some(1700600000)
    );
    assert_eq!(
        secret_json["codex_rate_limits"]["non_weekly_reset_at_epoch"].as_i64(),
        Some(1700003600)
    );

    let kv_path = cache_kv_path(&cache_root, "alpha");
    let kv = fs::read_to_string(&kv_path).expect("read kv");
    assert!(kv.contains("weekly_remaining=88"));
    assert!(kv.contains("non_weekly_remaining=94"));
}

#[test]
fn rate_limits_single_json_outputs_body() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");
    write_secret(&secrets, "alpha.json", Some("tok"));

    let cache_root = dir.path().join("cache_root");
    fs::create_dir_all(&cache_root).expect("cache root");

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, wham_usage_ok_body()),
    );

    let output = run(
        &["diag", "rate-limits", "--json", "alpha.json"],
        &[
            ("CODEX_SECRET_DIR", &secrets),
            ("ZSH_CACHE_DIR", &cache_root),
        ],
        &[
            ("CODEX_CHATGPT_BASE_URL", &server.url()),
            ("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false"),
            ("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
            ("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3"),
        ],
    );
    assert_exit(&output, 0);
    assert!(stdout(&output).contains("\"rate_limit\""));
}

#[test]
fn rate_limits_all_mode_renders_table() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");
    write_secret(&secrets, "alpha.json", Some("tok_a"));
    write_secret(&secrets, "beta.json", Some("tok_b"));

    let cache_root = dir.path().join("cache_root");
    fs::create_dir_all(&cache_root).expect("cache root");

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, wham_usage_ok_body()),
    );

    let output = run(
        &["diag", "rate-limits", "--all"],
        &[
            ("CODEX_SECRET_DIR", &secrets),
            ("ZSH_CACHE_DIR", &cache_root),
        ],
        &[
            ("CODEX_CHATGPT_BASE_URL", &server.url()),
            ("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false"),
            ("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
            ("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3"),
            ("TZ", "UTC"),
            ("NO_COLOR", "1"),
        ],
    );
    assert_exit(&output, 0);
    let out = stdout(&output);
    assert!(out.contains("🚦 Codex rate limits for all accounts"));
    assert!(out.contains("Name"));
    assert!(out.contains("alpha"));
    assert!(out.contains("beta"));
}

#[test]
fn rate_limits_async_falls_back_to_cache_in_debug_mode() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");
    write_secret(&secrets, "alpha.json", Some("tok_a"));
    write_secret(&secrets, "beta.json", None);

    let cache_root = dir.path().join("cache_root");
    fs::create_dir_all(&cache_root).expect("cache root");

    let fetched_at = now_epoch().saturating_sub(10).max(1);
    let kv_path = cache_kv_path(&cache_root, "beta");
    if let Some(parent) = kv_path.parent() {
        fs::create_dir_all(parent).expect("cache dir");
    }
    fs::write(
        &kv_path,
        format!(
            "fetched_at={fetched_at}\nnon_weekly_label=5h\nnon_weekly_remaining=1\nweekly_remaining=2\nweekly_reset_epoch=1700600000\n"
        ),
    )
    .expect("write cache kv");

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, wham_usage_ok_body()),
    );

    let output = run(
        &["diag", "rate-limits", "--async", "--debug"],
        &[
            ("CODEX_SECRET_DIR", &secrets),
            ("ZSH_CACHE_DIR", &cache_root),
        ],
        &[
            ("CODEX_CHATGPT_BASE_URL", &server.url()),
            ("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false"),
            ("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
            ("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3"),
            ("TZ", "UTC"),
            ("NO_COLOR", "1"),
        ],
    );
    assert_exit(&output, 0);

    assert!(stdout(&output).contains("🚦 Codex rate limits for all accounts"));
    assert!(stderr(&output).contains("falling back to cache for beta"));
    assert!(stderr(&output).contains("missing access_token"));
}

#[test]
fn rate_limits_clear_cache_removes_old_starship_cache_dir() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");
    write_secret(&secrets, "alpha.json", Some("tok"));

    let cache_root = dir.path().join("cache_root");
    let old_dir = cache_root.join("codex").join("starship-rate-limits");
    fs::create_dir_all(&old_dir).expect("cache dir");
    let junk = old_dir.join("junk.txt");
    fs::write(&junk, "junk").expect("write junk");
    assert!(junk.is_file());

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, wham_usage_ok_body()),
    );

    let output = run(
        &["diag", "rate-limits", "-c", "--one-line", "alpha.json"],
        &[
            ("CODEX_SECRET_DIR", &secrets),
            ("ZSH_CACHE_DIR", &cache_root),
        ],
        &[
            ("CODEX_CHATGPT_BASE_URL", &server.url()),
            ("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false"),
            ("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
            ("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3"),
            ("TZ", "UTC"),
            ("NO_COLOR", "1"),
        ],
    );
    assert_exit(&output, 0);

    assert!(!junk.exists());
    assert!(cache_kv_path(&cache_root, "alpha").is_file());
}
