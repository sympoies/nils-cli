//! `claude-cli diag rate-limits`: per-profile Claude OAuth usage in the shared
//! `diag rate-limits` result shape. Every request goes to a loopback fixture.

use crate::support::*;
use nils_test_support::cmd::{CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, RecordedRequest, TestServer};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const FUTURE_MS: i64 = 4_102_444_800_000; // 2100-01-01
const EXPIRED_MS: i64 = 1_000;
const SCHEMA: &str = "claude-cli.diag.rate-limits.v1";
const TOKEN_NAMES: &[&str] = &["alpha", "beta", "delta", "epsilon", "gamma", "zeta"];

const ALPHA_USAGE: &str = r#"{
  "five_hour": { "utilization": 25.0, "resets_at": "2023-11-14T22:13:20.000000+00:00" },
  "seven_day": { "utilization": 40.0, "resets_at": "2023-11-20T17:06:40+00:00" },
  "seven_day_opus": null
}"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    server: TestServer,
    alpha_limited: Arc<AtomicBool>,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let alpha_limited = Arc::new(AtomicBool::new(false));
        let limited = Arc::clone(&alpha_limited);
        let server = TestServer::new(move |request: &RecordedRequest| {
            let token = request
                .header_value("authorization")
                .unwrap_or_default()
                .trim_start_matches("Bearer ")
                .to_string();
            match token.as_str() {
                "access-alpha" if limited.load(Ordering::SeqCst) => {
                    HttpResponse::new(429, r#"{"error":{"message":"slow down"}}"#)
                }
                "access-alpha" => HttpResponse::new(200, ALPHA_USAGE),
                "access-beta" => HttpResponse::new(403, r#"{"error":{"message":"forbidden"}}"#),
                "access-delta" => HttpResponse::new(429, r#"{"error":{"message":"slow down"}}"#),
                "access-epsilon" => HttpResponse::new(502, "bad gateway"),
                "access-zeta" => HttpResponse::new(401, r#"{"error":{"message":"invalid"}}"#),
                _ => HttpResponse::new(404, "unknown token"),
            }
        })
        .expect("server");
        Self {
            _tmp: tmp,
            root,
            server,
            alpha_limited,
        }
    }

    fn secret_dir(&self) -> PathBuf {
        self.root.join("claude-secrets")
    }

    fn config_dir(&self) -> PathBuf {
        self.root.join("claude-config")
    }

    fn options(&self) -> CmdOptions {
        base_options(&self.root)
            .with_env("CLAUDE_SECRET_DIR", &path_str(&self.secret_dir()))
            .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
            .with_env_remove("CLAUDE_RATE_LIMITS_DEFAULT_ALL_ENABLED")
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/api/oauth/usage", self.server.url()),
            )
            .with_env("TZ", "UTC")
            .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
    }

    fn write_profile(&self, name: &str, access: &str, expires_at_ms: i64) {
        std::fs::create_dir_all(self.secret_dir()).expect("secret dir");
        write_json(
            &self.secret_dir().join(format!("{name}.json")),
            &json!({
                "claudeAiOauth": {
                    "accessToken": access,
                    "refreshToken": format!("refresh-{name}"),
                    "expiresAt": expires_at_ms,
                    "scopes": ["user:inference", "user:profile"],
                    "subscriptionType": "max"
                },
                "oauthAccount": {
                    "accountUuid": format!("{name}-uuid"),
                    "emailAddress": format!("{name}@example.com")
                }
            }),
        );
    }

    fn write_active_login(&self, access: &str, expires_at_ms: i64) {
        std::fs::create_dir_all(self.config_dir()).expect("config dir");
        write_json(
            &self.config_dir().join(".credentials.json"),
            &json!({
                "claudeAiOauth": {
                    "accessToken": access,
                    "refreshToken": "",
                    "expiresAt": expires_at_ms
                }
            }),
        );
    }

    fn run(&self, args: &[&str]) -> CmdOutput {
        self.run_with_options(args, &self.options())
    }

    fn run_with_options(&self, args: &[&str], options: &CmdOptions) -> CmdOutput {
        let output = run(args, options);
        for text in [stdout(&output), stderr(&output)] {
            for secret in TOKEN_NAMES
                .iter()
                .flat_map(|name| [format!("access-{name}"), format!("refresh-{name}")])
            {
                assert!(
                    !text.contains(&secret),
                    "diag rate-limits leaked a token: {text}"
                );
            }
        }
        output
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.server.take_requests()
    }
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).expect("json")).expect("write json");
}

fn alpha_result(source: &str) -> Value {
    let mut result = json!({
        "provider": "claude",
        "name": "alpha",
        "target_file": "alpha.json",
        "status": "ok",
        "ok": true,
        "source": source,
        "summary": {
            "non_weekly_label": "5h",
            "non_weekly_remaining": 75,
            "non_weekly_reset_epoch": 1_700_000_000,
            "weekly_remaining": 60,
            "weekly_reset_epoch": 1_700_500_000,
            "weekly_reset_local": "11-20 17:06 +00:00"
        },
        "windows": [
            {
                "label": "5h",
                "window_minutes": 300,
                "used_percent": 25,
                "remaining_percent": 75,
                "reset_at_epoch": 1_700_000_000
            },
            {
                "label": "Weekly",
                "window_minutes": 10080,
                "used_percent": 40,
                "remaining_percent": 60,
                "reset_at_epoch": 1_700_500_000
            }
        ]
    });
    // A network read always reports reset status; cached values never do.
    if source == "network" {
        result["limit_resets"] = json!({ "juniper_tide": null, "cedar_ember": null });
    }
    result
}

fn error_result(name: &str, reason: &str, code: &str) -> Value {
    json!({
        "provider": "claude",
        "name": name,
        "target_file": format!("{name}.json"),
        "status": "error",
        "ok": false,
        "source": "network",
        "reason_code": reason,
        "error": { "code": code }
    })
}

/// Drops the human error message so results compare on their stable fields.
fn without_error_messages(mut results: Value) -> Value {
    for result in results.as_array_mut().expect("results") {
        if let Some(error) = result.get_mut("error").and_then(Value::as_object_mut) {
            let message = error.remove("message").expect("error message");
            assert!(
                message
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("claude-rate-limits: ")
            );
        }
    }
    results
}

#[test]
fn diag_rate_limits_all_json_reports_every_profile_and_fails_when_one_has_no_window() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    fx.write_profile("beta", "access-beta", FUTURE_MS);
    fx.write_profile("delta", "access-delta", FUTURE_MS);
    fx.write_profile("epsilon", "access-epsilon", FUTURE_MS);
    fx.write_profile("gamma", "access-gamma", EXPIRED_MS);
    fx.write_profile("zeta", "access-zeta", FUTURE_MS);
    let before = std::fs::read(fx.secret_dir().join("alpha.json")).expect("alpha");

    let output = fx.run(&["diag", "rate-limits", "--all", "--format", "json"]);

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], SCHEMA);
    assert_eq!(payload["command"], "diag rate-limits");
    assert_eq!(payload["mode"], "all");
    assert_eq!(payload["ok"], false);
    let gamma = error_result("gamma", "auth_expired", "access-token-expired");
    assert_eq!(
        without_error_messages(payload["results"].clone()),
        json!([
            alpha_result("network"),
            error_result("beta", "permission_denied", "request-failed"),
            error_result("delta", "rate_limited", "request-failed"),
            error_result("epsilon", "service_unavailable", "request-failed"),
            gamma,
            error_result("zeta", "auth_expired", "request-failed"),
        ])
    );

    // The expired profile is never sent; nothing is refreshed or rewritten.
    let requests = fx.requests();
    assert_eq!(requests.len(), 5);
    for request in &requests {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/oauth/usage");
        assert_eq!(request.query.as_deref(), Some("at_wall=1&skip_spend=1"));
        assert_eq!(
            request.header_value("user-agent").as_deref(),
            Some("claude-cli/9.8.7 (external, cli)")
        );
        assert_eq!(
            request.header_value("anthropic-beta").as_deref(),
            Some("oauth-2025-04-20")
        );
        assert_ne!(
            request.header_value("authorization").as_deref(),
            Some("Bearer access-gamma")
        );
    }
    assert!(requests.iter().any(|request| {
        request.header_value("authorization").as_deref() == Some("Bearer access-alpha")
    }));
    assert_eq!(
        std::fs::read(fx.secret_dir().join("alpha.json")).expect("alpha"),
        before
    );
}

#[test]
fn diag_rate_limits_all_json_exits_zero_when_every_profile_has_windows() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "--all", "--json"]);

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["results"], json!([alpha_result("network")]));
}

#[test]
fn diag_rate_limits_all_json_reports_a_missing_secret_dir() {
    let fx = Fixture::new();

    let output = fx.run(&["diag", "rate-limits", "--all", "--format", "json"]);

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], SCHEMA);
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "secret-discovery-failed");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("CLAUDE_SECRET_DIR not found")
    );
}

#[test]
fn diag_rate_limits_named_profile_reports_a_single_result() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    fx.write_profile("beta", "access-beta", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "alpha", "--format", "json"]);
    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], SCHEMA);
    assert_eq!(payload["mode"], "single");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"], alpha_result("network"));

    let output = fx.run(&["diag", "rate-limits", "beta", "--format", "json"]);
    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["result"]["reason_code"], "permission_denied");

    let output = fx.run(&["diag", "rate-limits", "alpha"]);
    assert_exit(&output, 0);
    assert_eq!(
        stdout(&output),
        "Rate limits remaining\n5h 75% • 11-14 22:13\nWeekly 60% • 11-20 17:06\n"
    );

    let output = fx.run(&["diag", "rate-limits", "--one-line", "alpha"]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:75% W:60% 11-20 17:06\n");

    let output = fx.run(&["diag", "rate-limits", "../alpha"]);
    assert_exit(&output, 64);
}

#[test]
fn diag_rate_limits_json_rejects_an_invalid_profile_name_with_a_json_error() {
    let fx = Fixture::new();

    let output = fx.run(&["diag", "rate-limits", "--format", "json", "../x"]);

    assert_exit(&output, 64);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], SCHEMA);
    assert_eq!(payload["command"], "diag rate-limits");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "invalid-profile-name");
    assert!(fx.requests().is_empty());
}

#[test]
fn diag_rate_limits_without_a_target_reads_the_active_login() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    fx.write_active_login("access-alpha", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "--format", "json"]);
    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    let mut expected = alpha_result("network");
    expected["target_file"] = json!(".credentials.json");
    assert_eq!(payload["result"], expected);

    // An active login that no profile holds is reported as `active`.
    fx.write_active_login("access-beta", FUTURE_MS);
    let output = fx.run(&["diag", "rate-limits", "--format", "json"]);
    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["name"], "active");
    assert_eq!(payload["result"]["reason_code"], "permission_denied");

    fx.write_active_login("access-alpha", EXPIRED_MS);
    let requests_before = fx.requests().len();
    let output = fx.run(&["diag", "rate-limits", "--format", "json"]);
    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["reason_code"], "auth_expired");
    assert!(requests_before > 0);
    assert!(fx.requests().is_empty());
}

#[test]
fn diag_rate_limits_all_text_renders_the_shared_accounts_table() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    fx.write_profile("beta", "access-beta", FUTURE_MS);

    for args in [
        &["diag", "rate-limits", "--all"][..],
        &["diag", "rate-limits", "--async", "--jobs", "2"][..],
    ] {
        let output = fx.run(args);
        assert_exit(&output, 1);
        assert_eq!(
            stdout(&output),
            "\n🚦 Claude rate limits for all accounts\n\n\
Name                   5h     Left    Weekly     Left  Reset                 Resets\n\
-----------------------------------------------------------------------------------\n\
alpha                 75%   0h  0m       60%   0h  0m  11-20 17:06 +00:00         -\n\
beta                    -        -         -        -  -                          -\n"
        );
    }
}

#[test]
fn diag_rate_limits_default_all_env_reads_every_profile_when_no_target_is_given() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    fx.write_profile("beta", "access-beta", FUTURE_MS);
    fx.write_active_login("access-alpha", FUTURE_MS);
    let default_all = fx
        .options()
        .with_env("CLAUDE_RATE_LIMITS_DEFAULT_ALL_ENABLED", "true");

    let output = run(&["diag", "rate-limits"], &default_all);
    assert_exit(&output, 1);
    assert_eq!(
        stdout(&output),
        "\n🚦 Claude rate limits for all accounts\n\n\
Name                   5h     Left    Weekly     Left  Reset                 Resets\n\
-----------------------------------------------------------------------------------\n\
alpha                 75%   0h  0m       60%   0h  0m  11-20 17:06 +00:00         -\n\
beta                    -        -         -        -  -                          -\n"
    );

    // JSON and named targets keep their single-target meaning.
    let output = run(&["diag", "rate-limits", "--format", "json"], &default_all);
    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["target_file"], ".credentials.json");
    let output = run(
        &["diag", "rate-limits", "--one-line", "alpha"],
        &default_all,
    );
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:75% W:60% 11-20 17:06\n");

    // `--cached` stays a single cached read; the JSON run above cached it.
    let output = run(&["diag", "rate-limits", "--cached"], &default_all);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:75% W:60% 11-20 17:06\n");

    // Without the env, no target still reads only the active login.
    let output = fx.run(&["diag", "rate-limits", "--one-line"]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:75% W:60% 11-20 17:06\n");
}

#[test]
fn diag_rate_limits_async_json_falls_back_to_cache_and_cached_mode_stays_offline() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "--async", "--json"]);
    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["mode"], "async");
    assert_eq!(payload["results"], json!([alpha_result("network")]));

    fx.alpha_limited.store(true, Ordering::SeqCst);
    let output = fx.run(&["diag", "rate-limits", "--async", "--json"]);
    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["results"], json!([alpha_result("cache-fallback")]));

    let strict = fx
        .options()
        .with_env("CLAUDE_RATE_LIMITS_ASYNC_JSON_NO_CACHE_FALLBACK", "true");
    let output = fx.run_with_options(&["diag", "rate-limits", "--async", "--json"], &strict);
    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["results"][0]["ok"], false);
    assert_eq!(payload["results"][0]["reason_code"], "rate_limited");
    assert_eq!(payload["results"][0]["source"], "network");

    fx.requests();
    let output = fx.run(&["diag", "rate-limits", "--cached", "alpha"]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:75% W:60% 11-20 17:06\n");
    let output = fx.run(&["diag", "rate-limits", "--async", "--json", "--cached"]);
    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["results"][0]["source"], "cache");
    assert!(fx.requests().is_empty());
}

#[test]
fn diag_rate_limits_async_json_includes_unprofiled_active_login() {
    let fx = Fixture::new();
    fx.write_active_login("access-alpha", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "--async", "--format", "json"]);

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["mode"], "async");
    assert_eq!(payload["results"].as_array().unwrap().len(), 1);
    assert_eq!(payload["results"][0]["name"], "active");
    assert_eq!(payload["results"][0]["ok"], true);
}

#[test]
fn diag_rate_limits_async_json_includes_unmatched_active_login_once() {
    let fx = Fixture::new();
    fx.write_profile("beta", "access-beta", FUTURE_MS);
    fx.write_active_login("access-alpha", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "--async", "--format", "json"]);

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["results"].as_array().unwrap().len(), 2);
    assert!(
        payload["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| { result["name"] == "active" && result["ok"] == true })
    );
    assert!(
        payload["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| { result["name"] == "beta" && result["ok"] == false })
    );
}

#[test]
fn diag_rate_limits_async_json_keeps_active_identity_distinct_from_profile() {
    let fx = Fixture::new();
    fx.write_profile("active", "access-beta", FUTURE_MS);
    fx.write_active_login("access-alpha", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "--async", "--format", "json"]);

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .any(|result| { result["name"] == "active" && result["ok"] == false })
    );
    assert!(
        results
            .iter()
            .any(|result| { result["name"] == "active-login" && result["ok"] == true })
    );
}

#[test]
fn diag_rate_limits_async_json_fails_discovery_without_profiles_or_active_login() {
    let fx = Fixture::new();

    let output = fx.run(&["diag", "rate-limits", "--async", "--format", "json"]);
    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "secret-discovery-failed");

    std::fs::create_dir_all(fx.secret_dir()).expect("secret dir");
    let output = fx.run(&["diag", "rate-limits", "--async", "--format", "json"]);
    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "secret-discovery-failed");
}

#[test]
fn diag_rate_limits_async_json_does_not_hide_invalid_profile_storage() {
    let fx = Fixture::new();
    fx.write_active_login("access-alpha", FUTURE_MS);
    std::fs::write(fx.secret_dir(), "not a directory").expect("invalid secret dir");

    let output = fx.run(&["diag", "rate-limits", "--async", "--format", "json"]);

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "secret-discovery-failed");
}

#[test]
fn diag_rate_limits_rejects_codex_style_flag_conflicts() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);

    let output = fx.run(&["diag", "rate-limits", "--json", "--one-line", "alpha"]);
    assert_exit(&output, 64);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], SCHEMA);
    assert_eq!(payload["error"]["code"], "invalid-flag-combination");

    let output = fx.run(&["diag", "rate-limits", "--all", "alpha"]);
    assert_exit(&output, 64);

    let output = fx.run(&["diag", "rate-limits", "--watch"]);
    assert_exit(&output, 64);
    assert!(fx.requests().is_empty());
}

const RESETS_USAGE: &str = r#"{
  "five_hour": { "utilization": 25.0, "resets_at": "2023-11-14T22:13:20.000000+00:00" },
  "seven_day": { "utilization": 40.0, "resets_at": "2023-11-20T17:06:40+00:00" },
  "juniper_tide": {
    "eligible": true,
    "ineligible_reason": null,
    "in_experiment": true,
    "arm": "reset",
    "available": true,
    "next_available_at": "2026-10-08T01:00:00+00:00",
    "weekly_resets_at": "2026-10-07T01:00:00+00:00",
    "resets_per_week": 1,
    "event_props": { "cohort": "EVENT-PROPS-MARKER" }
  },
  "cedar_ember": {
    "eligible": true,
    "ineligible_reason": null,
    "at_limit": true,
    "exhausted": ["five_hour", "Not A Limit"],
    "grants": [
      {
        "id": "grant_a",
        "label": "Welcome reset",
        "resets_total": 2,
        "resets_left": 2,
        "starts_at": "2026-09-01T00:00:00Z",
        "ends_at": "2026-10-12T00:00:00Z",
        "clears": ["five_hour", "seven_day"],
        "paused": false,
        "usable_now": true,
        "percent_used": { "five_hour": 100 },
        "blocking": []
      },
      { "id": "BAD ID", "resets_left": 1 },
      { "id": "grant_b", "resets_left": -1 },
      {
        "id": "grant_c",
        "label": null,
        "resets_left": 1,
        "ends_at": "not a date",
        "clears": [],
        "paused": true,
        "use_requires_limit": false
      }
    ],
    "next_grant_id": "grant_a",
    "weekly_resets_at": "2026-10-07T01:00:00+00:00",
    "cooldown_until": null
  }
}"#;

fn resets_server(body: &'static str) -> TestServer {
    TestServer::new(move |_request: &RecordedRequest| HttpResponse::new(200, body)).expect("server")
}

fn resets_options(fx: &Fixture, server: &TestServer, endpoint_suffix: &str) -> CmdOptions {
    fx.options().with_env(
        "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
        &format!("{}/api/oauth/usage{endpoint_suffix}", server.url()),
    )
}

#[test]
fn diag_rate_limits_reports_normalized_limit_resets() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    let server = resets_server(RESETS_USAGE);

    let output = run(
        &["diag", "rate-limits", "alpha", "--format", "json"],
        &resets_options(&fx, &server, ""),
    );

    assert_exit(&output, 0);
    let text = stdout(&output);
    assert!(!text.contains("EVENT-PROPS-MARKER"), "{text}");
    assert!(!text.contains("access-alpha"), "{text}");
    let payload: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        payload["result"]["limit_resets"],
        json!({
            "juniper_tide": {
                "available": true,
                "eligible": true,
                "ineligible_reason": null,
                "arm": "reset",
                "resets_per_week": 1,
                "next_available_at": 1_791_421_200,
                "weekly_resets_at": 1_791_334_800
            },
            "cedar_ember": {
                "available": true,
                "eligible": true,
                "ineligible_reason": null,
                "at_limit": true,
                "exhausted": ["five_hour"],
                "next_grant_id": "grant_a",
                "grants": [
                    {
                        "id": "grant_a",
                        "label": "Welcome reset",
                        "resets_left": 2,
                        "resets_total": 2,
                        "starts_at": 1_788_220_800,
                        "ends_at": 1_791_763_200,
                        "clears": ["five_hour", "seven_day"],
                        "paused": false,
                        "usable_now": true,
                        "use_requires_limit": true
                    },
                    {
                        "id": "grant_c",
                        "label": null,
                        "resets_left": 1,
                        "resets_total": null,
                        "starts_at": null,
                        "ends_at": null,
                        "clears": [],
                        "paused": true,
                        "usable_now": false,
                        "use_requires_limit": false
                    }
                ],
                "cooldown_until": null,
                "weekly_resets_at": 1_791_334_800
            }
        })
    );
    // The windows are unaffected by the reset blocks.
    let mut expected = alpha_result("network");
    expected["limit_resets"] = payload["result"]["limit_resets"].clone();
    assert_eq!(payload["result"], expected);

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/oauth/usage");
    assert_eq!(requests[0].query.as_deref(), Some("at_wall=1&skip_spend=1"));
    assert_eq!(
        requests[0].header_value("user-agent").as_deref(),
        Some("claude-cli/9.8.7 (external, cli)")
    );
    assert_eq!(
        requests[0].header_value("anthropic-beta").as_deref(),
        Some("oauth-2025-04-20")
    );
}

#[test]
fn diag_rate_limits_degrades_malformed_reset_blocks_to_null_without_failing() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    let server = resets_server(
        r#"{
          "five_hour": { "utilization": 25.0, "resets_at": "2023-11-14T22:13:20.000000+00:00" },
          "seven_day": { "utilization": 40.0, "resets_at": "2023-11-20T17:06:40+00:00" },
          "juniper_tide": "surprise",
          "cedar_ember": {
            "eligible": "yes",
            "ineligible_reason": "Has Spaces!",
            "at_limit": "no",
            "exhausted": "five_hour",
            "grants": { "id": "grant_a" },
            "next_grant_id": "grant_missing",
            "weekly_resets_at": 17,
            "cooldown_until": "tomorrow"
          }
        }"#,
    );

    let output = run(
        &["diag", "rate-limits", "alpha", "--format", "json"],
        &resets_options(&fx, &server, ""),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["ok"], true);
    assert_eq!(
        payload["result"]["limit_resets"],
        json!({
            "juniper_tide": null,
            "cedar_ember": {
                "available": false,
                "eligible": false,
                "ineligible_reason": "unknown",
                "at_limit": null,
                "exhausted": [],
                "next_grant_id": null,
                "grants": [],
                "cooldown_until": null,
                "weekly_resets_at": null
            }
        })
    );
}

#[test]
fn diag_rate_limits_juniper_control_arm_and_ineligible_programs_are_unavailable() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    let server = resets_server(
        r#"{
          "five_hour": { "utilization": 25.0, "resets_at": null },
          "seven_day": null,
          "juniper_tide": { "eligible": true, "available": true, "arm": "control", "resets_per_week": 1 },
          "cedar_ember": {
            "eligible": false,
            "ineligible_reason": "tenure",
            "at_limit": false,
            "exhausted": [],
            "grants": [{ "id": "grant_a", "resets_left": 1 }],
            "next_grant_id": "grant_a",
            "weekly_resets_at": "2026-10-07T01:00:00+00:00",
            "cooldown_until": null
          }
        }"#,
    );

    let output = run(
        &["diag", "rate-limits", "alpha", "--format", "json"],
        &resets_options(&fx, &server, ""),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    let resets = &payload["result"]["limit_resets"];
    assert_eq!(resets["juniper_tide"]["available"], false);
    assert_eq!(resets["juniper_tide"]["eligible"], true);
    assert_eq!(resets["juniper_tide"]["arm"], "control");
    assert_eq!(resets["cedar_ember"]["available"], false);
    assert_eq!(resets["cedar_ember"]["ineligible_reason"], "tenure");
    assert_eq!(resets["cedar_ember"]["next_grant_id"], "grant_a");
}

#[test]
fn diag_rate_limits_appends_the_status_query_to_an_endpoint_override() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    let server = resets_server(ALPHA_USAGE);

    let output = run(
        &["diag", "rate-limits", "alpha", "--format", "json"],
        &resets_options(&fx, &server, "?tenant=demo"),
    );

    assert_exit(&output, 0);
    let requests = server.take_requests();
    assert_eq!(
        requests[0].query.as_deref(),
        Some("tenant=demo&at_wall=1&skip_spend=1")
    );
}

#[test]
fn diag_rate_limits_user_agent_follows_the_installed_claude_code_version() {
    let fx = Fixture::new();
    fx.write_profile("alpha", "access-alpha", FUTURE_MS);
    let server = resets_server(ALPHA_USAGE);
    let bin_dir = write_fake_claude(
        &fx.root,
        "#!/bin/sh\n[ \"$1\" = \"--version\" ] || exit 9\nprintf '2.3.45 (Claude Code)\\n'\n",
    );
    let user_agent = |options: CmdOptions| {
        let output = run(
            &["diag", "rate-limits", "alpha", "--format", "json"],
            &options,
        );
        assert_exit(&output, 0);
        server
            .take_requests()
            .pop()
            .and_then(|request| request.header_value("user-agent"))
            .expect("user agent")
    };

    // The installed Claude Code reports its version (an empty pin is unset).
    assert_eq!(
        user_agent(
            resets_options(&fx, &server, "")
                .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "")
                .with_fake_claude(&bin_dir)
        ),
        "claude-cli/2.3.45 (external, cli)"
    );
    // A malformed pin is ignored in favor of detection.
    assert_eq!(
        user_agent(
            resets_options(&fx, &server, "")
                .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "2.x")
                .with_fake_claude(&bin_dir)
        ),
        "claude-cli/2.3.45 (external, cli)"
    );
    // No Claude Code binary falls back to the pinned default.
    assert_eq!(
        user_agent(
            resets_options(&fx, &server, "").with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "")
        ),
        "claude-cli/2.1.284 (external, cli)"
    );
    // An explicit user-agent override still wins.
    assert_eq!(
        user_agent(
            resets_options(&fx, &server, "")
                .with_env("CLAUDE_PROMPT_SEGMENT_USER_AGENT", "custom-agent/1")
        ),
        "custom-agent/1"
    );
}
