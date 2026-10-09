//! `claude-cli auth reset-rate-limits`: redeem one Claude limit reset for a
//! stored profile. Every request goes to a loopback fixture; no test reaches a
//! real provider or a real profile.

use crate::support::*;
use nils_test_support::cmd::{CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, RecordedRequest, TestServer};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SCHEMA: &str = "claude-cli.auth.reset-rate-limits.v1";
const COMMAND: &str = "auth reset-rate-limits";
const FUTURE_MS: i64 = 4_102_444_800_000; // 2100-01-01
const EXPIRED_MS: i64 = 1_000;
const REQUEST_ID: &str = "0b5f4c1e-8d2a-4c7b-9e3f-2a1b0c9d8e7f";
const ACCESS: &str = "access-SECRET-alpha";
const ORG: &str = "org-SECRET-0001";
const ACCOUNT: &str = "account-SECRET-0001";
const GRANT: &str = "grant_secret_a";
const USAGE_PATH: &str = "/api/oauth/usage";
const RESET_PATH: &str = "/api/organizations/org-SECRET-0001/reset_rate_limits";
const UA: &str = "claude-cli/9.8.7 (external, cli)";
/// 2026-10-07T01:00:00Z and 2026-10-08T01:00:00Z.
const WEEKLY_EPOCH: i64 = 1_791_334_800;
const NEXT_EPOCH: i64 = 1_791_421_200;

fn status_body(juniper: Value, cedar: Value) -> String {
    json!({
        "five_hour": { "utilization": 100.0, "resets_at": "2026-10-01T15:00:00+00:00" },
        "seven_day": { "utilization": 40.0, "resets_at": "2026-10-07T01:00:00+00:00" },
        "juniper_tide": juniper,
        "cedar_ember": cedar,
    })
    .to_string()
}

fn juniper_available() -> Value {
    json!({
        "eligible": true, "ineligible_reason": null, "in_experiment": true, "arm": "reset",
        "available": true, "next_available_at": null,
        "weekly_resets_at": "2026-10-07T01:00:00+00:00", "resets_per_week": 1,
        "event_props": {}
    })
}

fn cedar_available() -> Value {
    json!({
        "eligible": true, "ineligible_reason": null, "at_limit": true, "exhausted": ["five_hour"],
        "grants": [{
            "id": GRANT, "label": "Welcome reset", "resets_total": 2, "resets_left": 2,
            "starts_at": "2026-09-01T00:00:00Z", "ends_at": "2026-10-12T00:00:00Z",
            "clears": ["five_hour"], "paused": false, "usable_now": true
        }],
        "next_grant_id": GRANT,
        "weekly_resets_at": "2026-10-07T01:00:00+00:00",
        "cooldown_until": null
    })
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    server: TestServer,
    status: Arc<Mutex<(u16, String)>>,
    reset: Arc<Mutex<(u16, String, u64)>>,
    reset_redirect: Arc<Mutex<Option<String>>>,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let status = Arc::new(Mutex::new((
            200,
            status_body(juniper_available(), cedar_available()),
        )));
        let reset = Arc::new(Mutex::new((
            200,
            json!({ "result": "reset" }).to_string(),
            0u64,
        )));
        let reset_redirect = Arc::new(Mutex::new(None::<String>));
        let (status_t, reset_t, reset_redirect_t) = (
            Arc::clone(&status),
            Arc::clone(&reset),
            Arc::clone(&reset_redirect),
        );
        let server = TestServer::new(move |request: &RecordedRequest| {
            match (request.method.as_str(), request.path.as_str()) {
                ("GET", USAGE_PATH) => {
                    let (code, body) = status_t.lock().expect("status").clone();
                    HttpResponse::new(code, body)
                }
                ("POST", RESET_PATH) => {
                    let (code, body, delay) = reset_t.lock().expect("reset").clone();
                    std::thread::sleep(Duration::from_millis(delay));
                    let response = HttpResponse::new(code, body);
                    match reset_redirect_t.lock().expect("redirect").clone() {
                        Some(location) => response.with_header("Location", &location),
                        None => response,
                    }
                }
                _ => HttpResponse::new(404, "unknown route"),
            }
        })
        .expect("server");
        let fx = Self {
            _tmp: tmp,
            root,
            server,
            status,
            reset,
            reset_redirect,
        };
        fx.write_profile("alpha", FUTURE_MS, Some(ORG));
        fx
    }

    fn secret_dir(&self) -> PathBuf {
        self.root.join("claude-secrets")
    }

    fn write_profile(&self, name: &str, expires_at_ms: i64, org: Option<&str>) {
        std::fs::create_dir_all(self.secret_dir()).expect("secret dir");
        let mut account = json!({ "accountUuid": ACCOUNT, "emailAddress": "alpha@example.com" });
        if let Some(org) = org {
            account["organizationUuid"] = json!(org);
        }
        write_json(
            &self.secret_dir().join(format!("{name}.json")),
            &json!({
                "claudeAiOauth": {
                    "accessToken": ACCESS,
                    "refreshToken": "refresh-SECRET-alpha",
                    "expiresAt": expires_at_ms,
                    "scopes": ["user:inference", "user:profile"],
                    "subscriptionType": "max"
                },
                "oauthAccount": account
            }),
        );
    }

    fn set_status(&self, code: u16, body: String) {
        *self.status.lock().expect("status") = (code, body);
    }

    fn set_reset(&self, code: u16, body: Value) {
        self.set_reset_raw(code, body.to_string(), 0);
    }

    fn set_reset_raw(&self, code: u16, body: String, delay_ms: u64) {
        *self.reset.lock().expect("reset") = (code, body, delay_ms);
    }

    fn set_reset_redirect(&self, location: &str) {
        self.set_reset_raw(307, "redirected".to_string(), 0);
        *self.reset_redirect.lock().expect("redirect") = Some(location.to_string());
    }

    fn options(&self) -> CmdOptions {
        base_options(&self.root)
            .with_env("CLAUDE_SECRET_DIR", &path_str(&self.secret_dir()))
            .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}{USAGE_PATH}", self.server.url()),
            )
            .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
            .with_env("CLAUDE_RATE_LIMITS_API_BASE_URL", "")
            .with_env("CLAUDE_RATE_LIMITS_RESET_MAX_TIME_SECONDS", "")
    }

    fn run_with(&self, args: &[&str], options: &CmdOptions) -> CmdOutput {
        let output = run(args, options);
        for text in [stdout(&output), stderr(&output)] {
            for secret in [
                ACCESS,
                "refresh-SECRET-alpha",
                ORG,
                ACCOUNT,
                GRANT,
                REQUEST_ID,
                "alpha@example.com",
                "claude-secrets",
                "SECRET",
            ] {
                assert!(
                    !text.contains(secret),
                    "reset-rate-limits output must redact sensitive values"
                );
            }
        }
        output
    }

    fn run(&self, args: &[&str]) -> CmdOutput {
        self.run_with(args, &self.options())
    }

    fn redeem(&self, program: &str) -> CmdOutput {
        self.run(&[
            "auth",
            "reset-rate-limits",
            "--yes",
            "--program",
            program,
            "--request-id",
            REQUEST_ID,
            "--format",
            "json",
            "alpha",
        ])
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.server.take_requests()
    }
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).expect("json")).expect("write json");
}

fn payload(output: &CmdOutput) -> Value {
    serde_json::from_str(&stdout(output)).expect("json stdout")
}

fn success(result: Value) -> Value {
    json!({ "schema_version": SCHEMA, "command": COMMAND, "ok": true, "result": result })
}

fn assert_status_get(request: &RecordedRequest) {
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, USAGE_PATH);
    assert_eq!(request.query.as_deref(), Some("at_wall=1&skip_spend=1"));
    assert_eq!(request.header_value("user-agent").as_deref(), Some(UA));
    assert_eq!(
        request.header_value("authorization").as_deref(),
        Some(format!("Bearer {ACCESS}").as_str())
    );
}

fn assert_reset_post(request: &RecordedRequest, body: &str) {
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, RESET_PATH);
    assert_eq!(request.query, None);
    assert_eq!(request.body_text(), body);
    assert_eq!(
        request.header_value("authorization").as_deref(),
        Some(format!("Bearer {ACCESS}").as_str())
    );
    assert_eq!(
        request.header_value("anthropic-beta").as_deref(),
        Some("oauth-2025-04-20")
    );
    assert_eq!(
        request.header_value("content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(request.header_value("user-agent").as_deref(), Some(UA));
}

fn assert_error(output: &CmdOutput, exit: i32, code: &str) -> Value {
    assert_exit(output, exit);
    let payload = payload(output);
    assert_eq!(payload["schema_version"], SCHEMA);
    assert_eq!(payload["command"], COMMAND);
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], code, "{payload}");
    assert!(payload.get("result").is_none());
    payload
}

#[test]
fn reset_rate_limits_redeems_the_next_cedar_grant_once() {
    let fx = Fixture::new();
    fx.set_reset(
        200,
        json!({
            "result": "reset", "reason": null, "resets_left": 1, "cleared": ["five_hour"],
            "weekly_resets_at": "2026-10-07T01:00:00+00:00", "cooldown_until": null
        }),
    );

    let output = fx.redeem("cedar_ember");

    assert_exit(&output, 0);
    assert_eq!(
        payload(&output),
        success(json!({
            "provider": "claude",
            "program": "cedar_ember",
            "outcome": "reset",
            "posted": true,
            "reason": null,
            "resets_left": 1,
            "next_available_at": null,
            "cooldown_until": null,
            "weekly_resets_at": WEEKLY_EPOCH
        }))
    );
    let requests = fx.requests();
    assert_eq!(requests.len(), 2);
    assert_status_get(&requests[0]);
    assert_reset_post(
        &requests[1],
        &format!(r#"{{"program":"cedar_ember","grant_id":"{GRANT}","request_id":"{REQUEST_ID}"}}"#),
    );
}

#[test]
fn reset_rate_limits_redeems_the_weekly_session_reset_with_only_the_program() {
    let fx = Fixture::new();
    fx.set_reset(
        200,
        json!({
            "result": "already_used",
            "next_available_at": "2026-10-08T01:00:00+00:00",
            "weekly_resets_at": "2026-10-07T01:00:00+00:00"
        }),
    );

    let output = fx.redeem("juniper_tide");

    assert_exit(&output, 0);
    assert_eq!(
        payload(&output),
        success(json!({
            "provider": "claude",
            "program": "juniper_tide",
            "outcome": "already_used",
            "posted": true,
            "reason": null,
            "resets_left": null,
            "next_available_at": NEXT_EPOCH,
            "cooldown_until": null,
            "weekly_resets_at": WEEKLY_EPOCH
        }))
    );
    let requests = fx.requests();
    assert_eq!(requests.len(), 2);
    assert_status_get(&requests[0]);
    assert_reset_post(&requests[1], r#"{"program":"juniper_tide"}"#);
}

#[test]
fn reset_rate_limits_reports_every_provider_outcome() {
    let fx = Fixture::new();
    for outcome in [
        "reset",
        "already_used",
        "not_limited",
        "cooldown",
        "ineligible",
        "unavailable",
    ] {
        fx.set_reset(
            200,
            json!({
                "result": outcome, "reason": "Not A Token", "resets_left": 0,
                "cooldown_until": "2026-10-08T01:00:00+00:00"
            }),
        );
        let output = fx.redeem("cedar_ember");
        assert_exit(&output, 0);
        let result = payload(&output)["result"].clone();
        assert_eq!(result["outcome"], outcome);
        assert_eq!(result["posted"], true);
        assert_eq!(result["reason"], "unknown");
        assert_eq!(result["resets_left"], 0);
        assert_eq!(result["cooldown_until"], NEXT_EPOCH);
    }

    fx.set_reset(200, json!({ "result": "exploded" }));
    let output = fx.redeem("cedar_ember");
    assert_error(&output, 3, "invalid-provider-response");

    fx.set_reset_raw(200, "not json".to_string(), 0);
    let output = fx.redeem("juniper_tide");
    assert_error(&output, 3, "invalid-provider-response");
}

#[test]
fn reset_rate_limits_reports_an_unavailable_program_without_posting() {
    let fx = Fixture::new();
    fx.set_status(
        200,
        status_body(
            json!({ "eligible": false, "ineligible_reason": "not_at_wall", "available": false, "resets_per_week": 1 }),
            json!({
                "eligible": false, "ineligible_reason": "tenure", "at_limit": false, "exhausted": [],
                "grants": [], "next_grant_id": null,
                "weekly_resets_at": "2026-10-07T01:00:00+00:00", "cooldown_until": null
            }),
        ),
    );

    let output = fx.redeem("juniper_tide");
    assert_exit(&output, 0);
    assert_eq!(
        payload(&output),
        success(json!({
            "provider": "claude",
            "program": "juniper_tide",
            "outcome": "unavailable",
            "posted": false,
            "reason": "not_at_wall",
            "resets_left": null,
            "next_available_at": null,
            "cooldown_until": null,
            "weekly_resets_at": null
        }))
    );
    let output = fx.redeem("cedar_ember");
    assert_exit(&output, 0);
    assert_eq!(
        payload(&output),
        success(json!({
            "provider": "claude",
            "program": "cedar_ember",
            "outcome": "unavailable",
            "posted": false,
            "reason": "tenure",
            "resets_left": null,
            "next_available_at": null,
            "cooldown_until": null,
            "weekly_resets_at": WEEKLY_EPOCH
        }))
    );

    // A control arm, a paused next grant, and an absent program never post.
    let mut paused = cedar_available();
    paused["grants"][0]["paused"] = json!(true);
    fx.set_status(
        200,
        status_body(
            json!({ "eligible": true, "available": true, "arm": "control" }),
            paused,
        ),
    );
    for program in ["juniper_tide", "cedar_ember"] {
        let output = fx.redeem(program);
        assert_exit(&output, 0);
        assert_eq!(payload(&output)["result"]["outcome"], "unavailable");
        assert_eq!(payload(&output)["result"]["posted"], false);
    }
    fx.set_status(200, json!({ "five_hour": null }).to_string());
    let output = fx.redeem("cedar_ember");
    assert_exit(&output, 0);
    assert_eq!(payload(&output)["result"]["outcome"], "unavailable");
    assert_eq!(payload(&output)["result"]["reason"], Value::Null);

    let requests = fx.requests();
    assert_eq!(requests.len(), 5);
    assert!(requests.iter().all(|request| request.method == "GET"));
}

#[test]
fn reset_rate_limits_refuses_an_expired_or_unscoped_profile_before_any_request() {
    let fx = Fixture::new();
    fx.write_profile("alpha", EXPIRED_MS, Some(ORG));
    let before = std::fs::read(fx.secret_dir().join("alpha.json")).expect("profile");

    let payload = assert_error(&fx.redeem("cedar_ember"), 2, "claude-auth-required");
    assert_eq!(payload["error"]["details"]["reason_code"], "auth_expired");
    assert_eq!(payload["error"]["details"]["retryable"], false);
    // The token is never refreshed or rewritten.
    assert_eq!(
        std::fs::read(fx.secret_dir().join("alpha.json")).expect("profile"),
        before
    );

    fx.write_profile("alpha", FUTURE_MS, None);
    assert_error(&fx.redeem("cedar_ember"), 1, "organization-unknown");

    assert!(fx.requests().is_empty());
}

#[test]
fn reset_rate_limits_requires_confirmation_and_a_request_id_before_any_request() {
    let fx = Fixture::new();
    let base = ["auth", "reset-rate-limits", "--program", "cedar_ember"];
    let with = |extra: &[&'static str]| -> Vec<&str> {
        base.iter().copied().chain(extra.iter().copied()).collect()
    };

    let output = fx.run(&with(&[
        "--request-id",
        REQUEST_ID,
        "--format",
        "json",
        "alpha",
    ]));
    assert_error(&output, 64, "confirmation-required");

    let output = fx.run(&with(&["--yes", "--json", "alpha"]));
    assert_error(&output, 64, "request-id-required");

    for bad in [
        "0B5F4C1E-8D2A-4C7B-9E3F-2A1B0C9D8E7F",
        "0b5f4c1e8d2a4c7b9e3f2a1b0c9d8e7f",
        "not-a-uuid",
    ] {
        let output = fx.run(&with(&[
            "--yes",
            "--request-id",
            bad,
            "--format",
            "json",
            "alpha",
        ]));
        assert_error(&output, 64, "invalid-request-id");
    }

    let output = fx.run(&with(&[
        "--yes",
        "--request-id",
        REQUEST_ID,
        "--json",
        "../alpha",
    ]));
    assert_error(&output, 64, "invalid-profile-name");

    let output = fx.run(&with(&[
        "--yes",
        "--request-id",
        REQUEST_ID,
        "--json",
        "missing",
    ]));
    assert_error(&output, 1, "profile-not-found");

    // Text mode reports the same refusal on stderr.
    let output = fx.run(&with(&["--request-id", REQUEST_ID, "alpha"]));
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--yes"), "{}", stderr(&output));

    // clap owns the program grammar.
    let output = fx.run(&[
        "auth",
        "reset-rate-limits",
        "--yes",
        "--program",
        "free_lunch",
        "--request-id",
        REQUEST_ID,
        "alpha",
    ]);
    assert_exit(&output, 64);

    assert!(fx.requests().is_empty());
}

#[test]
fn reset_rate_limits_maps_http_failures_without_retrying_the_post() {
    let fx = Fixture::new();
    for (status, exit, code, reason, retryable) in [
        (401, 2, "claude-auth-required", "auth_expired", false),
        (403, 2, "claude-auth-required", "permission_denied", false),
        (429, 3, "provider-unavailable", "rate_limited", true),
        (503, 3, "provider-unavailable", "service_unavailable", true),
    ] {
        fx.set_reset(
            status,
            json!({ "error": { "message": "SECRET upstream text" } }),
        );
        let payload = assert_error(&fx.redeem("cedar_ember"), exit, code);
        assert_eq!(payload["error"]["details"]["reason_code"], reason);
        assert_eq!(payload["error"]["details"]["retryable"], retryable);
        let requests = fx.requests();
        assert_eq!(requests.len(), 2, "status {status}");
        assert_eq!(requests[1].method, "POST");
    }

    // Like Claude Code, any other non-2xx answer is a rejection, even when its
    // body reads as a result: only a 2xx response reports an outcome.
    for status in [400, 404, 409] {
        fx.set_reset(status, json!({ "result": "reset", "resets_left": 0 }));
        let payload = assert_error(&fx.redeem("cedar_ember"), 3, "provider-rejected");
        assert_eq!(
            payload["error"]["details"]["retryable"], false,
            "status {status}"
        );
        assert!(
            payload.get("result").is_none(),
            "status {status}: {payload}"
        );
        let requests = fx.requests();
        assert_eq!(requests.len(), 2, "status {status}");
        assert_eq!(requests[1].method, "POST");
    }

    // A failed status read never posts.
    fx.set_status(403, "{}".to_string());
    assert_error(&fx.redeem("juniper_tide"), 2, "claude-auth-required");
    fx.set_status(200, "not json".to_string());
    assert_error(&fx.redeem("juniper_tide"), 3, "invalid-provider-response");
    let requests = fx.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.method == "GET"));
}

#[test]
fn reset_rate_limits_does_not_follow_reset_redirects() {
    let fx = Fixture::new();
    let redirect_target = TestServer::new(|_request: &RecordedRequest| {
        HttpResponse::new(200, r#"{"result":"reset"}"#)
    })
    .expect("redirect target");
    fx.set_reset_redirect(&format!("{}/capture", redirect_target.url()));

    let output = fx.run(&[
        "auth",
        "reset-rate-limits",
        "--yes",
        "--program",
        "juniper_tide",
        "--request-id",
        REQUEST_ID,
        "--format",
        "json",
        "alpha",
    ]);

    assert_error(&output, 3, "provider-rejected");
    assert!(
        redirect_target.take_requests().is_empty(),
        "the reset request must not be replayed to a redirect target"
    );
}

#[test]
fn reset_rate_limits_times_out_as_a_retryable_unknown_result() {
    let fx = Fixture::new();
    fx.set_reset_raw(200, json!({ "result": "reset" }).to_string(), 2_500);
    let options = fx
        .options()
        .with_env("CLAUDE_RATE_LIMITS_RESET_MAX_TIME_SECONDS", "1");

    let output = fx.run_with(
        &[
            "auth",
            "reset-rate-limits",
            "-y",
            "--program",
            "juniper_tide",
            "--request-id",
            REQUEST_ID,
            "--json",
            "alpha",
        ],
        &options,
    );

    let payload = assert_error(&output, 3, "provider-unavailable");
    assert_eq!(payload["error"]["details"]["reason_code"], "timeout");
    assert_eq!(payload["error"]["details"]["retryable"], true);
    // The server records the abandoned POST once its delayed handler returns.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut requests = fx.requests();
    while requests.len() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        requests.extend(fx.requests());
    }
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].method, "POST");
}

#[test]
fn reset_rate_limits_refuses_a_cleartext_non_loopback_api_base_url() {
    let fx = Fixture::new();
    for base in ["http://api.example.invalid", "ftp://127.0.0.1", "not a url"] {
        let options = fx
            .options()
            .with_env("CLAUDE_RATE_LIMITS_API_BASE_URL", base);
        let output = fx.run_with(
            &[
                "auth",
                "reset-rate-limits",
                "--yes",
                "--program",
                "juniper_tide",
                "--request-id",
                REQUEST_ID,
                "--format",
                "json",
                "alpha",
            ],
            &options,
        );
        assert_error(&output, 1, "endpoint-invalid");
        assert!(
            fx.requests().is_empty(),
            "{base} must not send the token anywhere"
        );
    }
}

#[test]
fn reset_rate_limits_posts_to_the_configured_api_base_url() {
    let fx = Fixture::new();
    let other = TestServer::new(|_request: &RecordedRequest| {
        HttpResponse::new(200, r#"{"result":"not_limited"}"#)
    })
    .expect("server");
    let options = fx
        .options()
        .with_env("CLAUDE_RATE_LIMITS_API_BASE_URL", &other.url());

    let output = fx.run_with(
        &[
            "auth",
            "reset-rate-limits",
            "--yes",
            "--program",
            "juniper_tide",
            "--request-id",
            REQUEST_ID,
            "--format",
            "json",
            "alpha",
        ],
        &options,
    );

    assert_exit(&output, 0);
    assert_eq!(payload(&output)["result"]["outcome"], "not_limited");
    let requests = fx.requests();
    assert_eq!(requests.len(), 1);
    assert_status_get(&requests[0]);
    let posts = other.take_requests();
    assert_eq!(posts.len(), 1);
    assert_reset_post(&posts[0], r#"{"program":"juniper_tide"}"#);
}

#[test]
fn reset_rate_limits_text_output_names_the_outcome() {
    let fx = Fixture::new();
    fx.set_reset(200, json!({ "result": "reset", "resets_left": 1 }));

    let output = fx.run(&[
        "auth",
        "reset-rate-limits",
        "--yes",
        "--program",
        "cedar_ember",
        "--request-id",
        REQUEST_ID,
        "alpha",
    ]);

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "Used a granted limit reset (1 left).\n");
}

#[test]
fn reset_rate_limits_usage_status_403_rate_limit_is_provider_unavailable() {
    let fx = Fixture::new();
    *fx.status.lock().unwrap() = (403, "too many requests".to_string());
    let output = fx.redeem("juniper_tide");
    assert_exit(&output, 3);
    let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["error"]["code"], "provider-unavailable");
    assert_eq!(payload["error"]["details"]["reason_code"], "rate_limited");
    let requests = fx.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, USAGE_PATH);
}
