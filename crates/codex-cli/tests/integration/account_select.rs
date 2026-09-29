use chrono::Utc;
use codex_cli::account::select::{
    CandidateCapacity, Capacity, CapacitySource, MIN_REMAINING_PERCENT, RateLimitSnapshot,
    SelectError, Strategy, effective_origin, select,
};
use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Fixture rate-limit snapshots (pure selection library)
// ---------------------------------------------------------------------------

fn snapshot(five_hour: Option<i64>, weekly: Option<i64>) -> RateLimitSnapshot {
    RateLimitSnapshot {
        fetched_at_epoch: Some(1_900_000_000),
        non_weekly_label: five_hour.map(|_| "5h".to_string()),
        non_weekly_remaining: five_hour,
        non_weekly_reset_epoch: five_hour.map(|_| 1_900_018_000),
        weekly_remaining: weekly,
        weekly_reset_epoch: weekly.map(|_| 1_900_604_800),
    }
}

fn candidate(name: &str, snap: Option<RateLimitSnapshot>) -> CandidateCapacity {
    CandidateCapacity::from_snapshot(name, snap.as_ref(), CapacitySource::Cache)
}

fn available(name: &str) -> CandidateCapacity {
    candidate(name, Some(snapshot(Some(40), Some(70))))
}

fn exhausted(name: &str) -> CandidateCapacity {
    candidate(name, Some(snapshot(Some(0), Some(55))))
}

fn unknown(name: &str) -> CandidateCapacity {
    candidate(name, None)
}

fn as_default(mut value: CandidateCapacity) -> CandidateCapacity {
    value.default = true;
    value
}

fn as_excluded(mut value: CandidateCapacity) -> CandidateCapacity {
    value.excluded = true;
    value
}

#[test]
fn select_snapshot_classification_uses_documented_threshold() {
    assert_eq!(MIN_REMAINING_PERCENT, 1);
    let at_threshold = candidate("a", Some(snapshot(Some(1), Some(1))));
    assert_eq!(at_threshold.capacity, Capacity::Available);
    assert_eq!(at_threshold.min_remaining_percent, Some(1));

    let weekly_exhausted = candidate("b", Some(snapshot(Some(90), Some(0))));
    assert_eq!(weekly_exhausted.capacity, Capacity::Exhausted);
    assert_eq!(weekly_exhausted.min_remaining_percent, Some(0));

    let weekly_only = candidate("c", Some(snapshot(None, Some(12))));
    assert_eq!(weekly_only.capacity, Capacity::Available);
    assert_eq!(weekly_only.windows.len(), 1);
    assert_eq!(weekly_only.windows[0].label, "weekly");

    let empty = candidate("d", Some(snapshot(None, None)));
    assert_eq!(empty.capacity, Capacity::Unknown);
    assert_eq!(empty.source, CapacitySource::None);

    let missing = unknown("e");
    assert_eq!(missing.capacity, Capacity::Unknown);
    assert!(missing.windows.is_empty());
}

#[test]
fn select_current_default_ignores_capacity() {
    let candidates = vec![as_default(exhausted("alpha")), available("beta")];
    assert_eq!(
        select(Strategy::CurrentDefault, &candidates, None).unwrap(),
        "alpha"
    );
}

#[test]
fn select_current_default_without_default_fails() {
    let candidates = vec![available("alpha"), available("beta")];
    let error = select(Strategy::CurrentDefault, &candidates, None).unwrap_err();
    assert_eq!(error, SelectError::DefaultUnavailable);
    assert_eq!(error.code(), "default-account-unavailable");

    let excluded_default = vec![as_excluded(as_default(available("alpha")))];
    assert_eq!(
        select(Strategy::CurrentDefault, &excluded_default, None).unwrap_err(),
        SelectError::DefaultUnavailable
    );
}

#[test]
fn select_next_with_capacity_rotates_after_origin_in_nickname_order() {
    let candidates = vec![
        as_default(available("alpha")),
        exhausted("bravo"),
        as_excluded(available("charlie")),
        unknown("delta"),
        available("echo"),
    ];
    // The origin defaults to the current default and is itself skipped.
    assert_eq!(
        select(Strategy::NextWithCapacity, &candidates, None).unwrap(),
        "echo"
    );
    // An explicit origin wraps around the sorted ring.
    assert_eq!(
        select(Strategy::NextWithCapacity, &candidates, Some("echo")).unwrap(),
        "alpha"
    );
}

#[test]
fn select_is_deterministic_regardless_of_input_order() {
    let ordered = vec![
        as_default(exhausted("alpha")),
        available("bravo"),
        available("charlie"),
    ];
    let shuffled = vec![
        available("charlie"),
        as_default(exhausted("alpha")),
        available("bravo"),
    ];
    for strategy in [Strategy::NextWithCapacity, Strategy::DefaultWithCapacity] {
        assert_eq!(select(strategy, &ordered, None).unwrap(), "bravo");
        assert_eq!(select(strategy, &shuffled, None).unwrap(), "bravo");
    }
}

#[test]
fn select_effective_origin_matches_the_origin_select_uses() {
    let candidates = vec![
        available("alpha"),
        as_default(available("bravo")),
        available("charlie"),
    ];
    assert_eq!(
        effective_origin(Strategy::NextWithCapacity, &candidates, None),
        Some("bravo")
    );
    assert_eq!(
        effective_origin(Strategy::NextWithCapacity, &candidates, Some("charlie")),
        Some("charlie")
    );
    assert_eq!(
        effective_origin(Strategy::DefaultWithCapacity, &candidates, Some("charlie")),
        None
    );
    assert_eq!(
        select(Strategy::NextWithCapacity, &candidates, None).unwrap(),
        "charlie"
    );
}

#[test]
fn select_next_with_capacity_without_origin_starts_at_first_nickname() {
    let candidates = vec![exhausted("alpha"), available("bravo"), available("charlie")];
    assert_eq!(
        select(Strategy::NextWithCapacity, &candidates, None).unwrap(),
        "bravo"
    );
}

#[test]
fn select_next_with_capacity_rejects_unknown_origin() {
    let candidates = vec![available("alpha")];
    let error = select(Strategy::NextWithCapacity, &candidates, Some("zulu")).unwrap_err();
    assert_eq!(error, SelectError::UnknownOrigin);
    assert_eq!(error.code(), "unknown-origin");
}

#[test]
fn select_default_with_capacity_keeps_available_default() {
    let candidates = vec![available("alpha"), as_default(available("bravo"))];
    assert_eq!(
        select(Strategy::DefaultWithCapacity, &candidates, None).unwrap(),
        "bravo"
    );
}

#[test]
fn select_default_with_capacity_fails_over_from_exhausted_default() {
    let candidates = vec![
        available("alpha"),
        as_default(exhausted("bravo")),
        unknown("charlie"),
    ];
    assert_eq!(
        select(Strategy::DefaultWithCapacity, &candidates, None).unwrap(),
        "alpha"
    );
}

#[test]
fn select_default_with_capacity_fails_over_from_excluded_default() {
    let candidates = vec![
        as_excluded(as_default(available("alpha"))),
        available("bravo"),
    ];
    assert_eq!(
        select(Strategy::DefaultWithCapacity, &candidates, None).unwrap(),
        "bravo"
    );
}

#[test]
fn select_default_with_capacity_refuses_unknown_default() {
    let candidates = vec![as_default(unknown("alpha")), available("bravo")];
    let error = select(Strategy::DefaultWithCapacity, &candidates, None).unwrap_err();
    assert_eq!(error, SelectError::DefaultCapacityUnknown);
    assert_eq!(error.code(), "default-capacity-unknown");

    let no_default = vec![available("alpha")];
    assert_eq!(
        select(Strategy::DefaultWithCapacity, &no_default, None).unwrap_err(),
        SelectError::DefaultUnavailable
    );
}

#[test]
fn select_reports_no_capacity_when_every_candidate_is_ineligible() {
    let candidates = vec![
        as_default(exhausted("alpha")),
        exhausted("bravo"),
        unknown("charlie"),
        as_excluded(available("delta")),
    ];
    for strategy in [Strategy::NextWithCapacity, Strategy::DefaultWithCapacity] {
        let error = select(strategy, &candidates, None).unwrap_err();
        assert_eq!(error, SelectError::NoCapacity);
        assert_eq!(error.code(), "no-account-with-capacity");
    }
}

// ---------------------------------------------------------------------------
// CLI contract against cached and network fixture snapshots
// ---------------------------------------------------------------------------

const HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
// {"sub":"user_123","email":"alpha@example.com","https://api.openai.com/auth":{"chatgpt_user_id":"user_123","email":"alpha@example.com"}}
const PAYLOAD_ALPHA: &str = "eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19";

struct Fixture {
    _dir: tempfile::TempDir,
    home: PathBuf,
    secrets: PathBuf,
    cache_root: PathBuf,
    auth_file: PathBuf,
}

impl Fixture {
    fn new(profiles: &[&str], default: &str) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let home = dir.path().join("home");
        let secrets = dir.path().join("secrets");
        let cache_root = dir.path().join("cache");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&secrets).expect("secrets");
        fs::create_dir_all(&cache_root).expect("cache");
        for profile in profiles {
            fs::write(
                secrets.join(format!("{profile}.json")),
                secret_json(profile),
            )
            .expect("secret");
        }
        let auth_file = dir.path().join("auth.json");
        fs::write(&auth_file, secret_json(default)).expect("auth");
        Self {
            _dir: dir,
            home,
            secrets,
            cache_root,
            auth_file,
        }
    }

    fn write_cache(&self, profile: &str, five_hour: i64, weekly: i64) {
        let now = Utc::now().timestamp();
        self.write_cache_at(profile, now, five_hour, weekly);
    }

    fn write_cache_at(&self, profile: &str, fetched_at: i64, five_hour: i64, weekly: i64) {
        let path = cache_kv_path(&self.cache_root, profile);
        fs::create_dir_all(path.parent().unwrap()).expect("cache dir");
        fs::write(
            path,
            format!(
                "fetched_at={fetched_at}\nnon_weekly_label=5h\nnon_weekly_remaining={five_hour}\nnon_weekly_reset_epoch={}\nweekly_remaining={weekly}\nweekly_reset_epoch={}",
                fetched_at + 3_600,
                fetched_at + 500_000
            ),
        )
        .expect("cache kv");
    }

    fn run(&self, server: &LoopbackServer, args: &[&str]) -> CmdOutput {
        self.run_with_env(server, args, &[])
    }

    fn run_with_env(
        &self,
        server: &LoopbackServer,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> CmdOutput {
        let mut options = CmdOptions::default()
            .with_env("HOME", &self.home.to_string_lossy())
            .with_env("CODEX_SECRET_DIR", &self.secrets.to_string_lossy())
            .with_env("CODEX_AUTH_FILE", &self.auth_file.to_string_lossy())
            .with_env("ZSH_CACHE_DIR", &self.cache_root.to_string_lossy())
            .with_env("CODEX_CHATGPT_BASE_URL", &server.url())
            .with_env("CODEX_AUTO_REFRESH_ENABLED", "false")
            .with_env("CODEX_RATE_LIMITS_CACHE_TTL", "5m")
            .with_env("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3")
            .with_env_remove("CODEX_RATE_LIMITS_CACHE_ALLOW_STALE")
            .with_env_remove("CODEX_HOME");
        for (key, value) in extra {
            options = options.with_env(key, value);
        }
        cmd::run_with(&bin::resolve("codex-cli"), args, &options)
    }
}

fn secret_json(profile: &str) -> String {
    format!(
        r#"{{"tokens":{{"access_token":"{HEADER}.{PAYLOAD_ALPHA}.sig-{profile}","id_token":"{HEADER}.{PAYLOAD_ALPHA}.sig-{profile}","refresh_token":"refresh-{profile}","account_id":"acct-{profile}"}},"last_refresh":"2026-09-01T00:00:00Z"}}"#
    )
}

fn cache_kv_path(cache_root: &Path, key: &str) -> PathBuf {
    cache_root
        .join("codex")
        .join("prompt-segment-rate-limits")
        .join(format!("{key}.kv"))
}

fn usage_body(five_hour_used: i64, weekly_used: i64) -> String {
    let now = Utc::now().timestamp();
    serde_json::json!({
        "plan_type": "plus",
        "rate_limit": {
            "primary_window": {
                "limit_window_seconds": 18_000,
                "used_percent": five_hour_used,
                "reset_at": now + 3_600
            },
            "secondary_window": {
                "limit_window_seconds": 604_800,
                "used_percent": weekly_used,
                "reset_at": now + 500_000
            }
        }
    })
    .to_string()
}

fn json(output: &CmdOutput) -> Value {
    serde_json::from_str(&output.stdout_text()).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}):\n{}\nstderr:\n{}",
            output.stdout_text(),
            output.stderr_text()
        )
    })
}

fn assert_no_sensitive_fields(output: &CmdOutput) {
    let stdout = output.stdout_text();
    for needle in [
        "acct-",
        "refresh-",
        "sig-",
        HEADER,
        "alpha@example.com",
        "user_123",
        "/secrets",
        "auth.json",
    ] {
        assert!(!stdout.contains(needle), "leaked {needle:?} in:\n{stdout}");
    }
}

fn candidate_names(payload: &Value, pointer: &str) -> Vec<String> {
    payload
        .pointer(pointer)
        .and_then(Value::as_array)
        .expect("candidates")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn account_select_next_with_capacity_uses_cached_snapshots_without_network() {
    let fixture = Fixture::new(&["alpha", "bravo", "charlie"], "alpha");
    fixture.write_cache("alpha", 30, 60);
    fixture.write_cache("bravo", 0, 80);
    fixture.write_cache("charlie", 25, 45);
    let server = LoopbackServer::new().expect("server");

    let output = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "next-with-capacity",
            "--format",
            "json",
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    assert!(
        server.take_requests().is_empty(),
        "fresh cache must not fetch"
    );
    let payload = json(&output);
    assert_eq!(payload["schema_version"], "codex-cli.account.select.v1");
    assert_eq!(payload["command"], "account select");
    assert_eq!(payload["ok"], true);
    let result = &payload["result"];
    assert_eq!(result["strategy"], "next-with-capacity");
    assert_eq!(result["selected"], "charlie");
    assert_eq!(result["default_account"], "alpha");
    assert_eq!(result["origin"], "alpha");
    assert_eq!(result["thresholds"]["min_remaining_percent"], 1);
    assert_eq!(result["cache_ttl_seconds"], 300);
    assert_eq!(
        candidate_names(&payload, "/result/candidates"),
        vec!["alpha", "bravo", "charlie"]
    );
    let bravo = &result["candidates"][1];
    assert_eq!(bravo["capacity"], "exhausted");
    assert_eq!(bravo["source"], "cache");
    assert_eq!(bravo["default"], false);
    assert_eq!(bravo["excluded"], false);
    assert_eq!(bravo["min_remaining_percent"], 0);
    assert_eq!(bravo["windows"][0]["label"], "5h");
    assert_eq!(bravo["windows"][0]["remaining_percent"], 0);
    assert_eq!(bravo["windows"][1]["label"], "weekly");
    assert_eq!(bravo["windows"][1]["remaining_percent"], 80);
    assert_eq!(result["candidates"][0]["default"], true);
    assert_no_sensitive_fields(&output);
}

#[test]
fn account_select_accepts_broker_strategy_spelling_and_exclusions() {
    let fixture = Fixture::new(&["alpha", "bravo", "charlie"], "alpha");
    fixture.write_cache("alpha", 30, 60);
    fixture.write_cache("bravo", 50, 80);
    fixture.write_cache("charlie", 25, 45);
    let server = LoopbackServer::new().expect("server");

    let output = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "next_with_capacity",
            "--after",
            "alpha",
            "--exclude",
            "bravo",
            "--format",
            "json",
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let payload = json(&output);
    assert_eq!(payload["result"]["strategy"], "next-with-capacity");
    assert_eq!(payload["result"]["selected"], "charlie");
    assert_eq!(payload["result"]["candidates"][1]["excluded"], true);
}

#[test]
fn account_select_current_default_is_cache_only() {
    let fixture = Fixture::new(&["alpha", "bravo"], "bravo");
    fixture.write_cache("alpha", 30, 60);
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, usage_body(10, 10)),
    );

    let output = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "current-default",
            "--format",
            "json",
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    assert!(
        server.take_requests().is_empty(),
        "current-default must not fetch"
    );
    let payload = json(&output);
    assert_eq!(payload["result"]["selected"], "bravo");
    assert_eq!(payload["result"]["origin"], Value::Null);
    let bravo = &payload["result"]["candidates"][1];
    assert_eq!(bravo["capacity"], "unknown");
    assert_eq!(bravo["source"], "none");
}

#[test]
fn account_select_text_output_prints_only_the_nickname() {
    let fixture = Fixture::new(&["alpha", "bravo"], "alpha");
    fixture.write_cache("alpha", 0, 60);
    fixture.write_cache("bravo", 20, 60);
    let server = LoopbackServer::new().expect("server");

    let output = fixture.run(
        &server,
        &["account", "select", "--strategy", "default-with-capacity"],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    assert_eq!(output.stdout_text(), "bravo\n");
}

#[test]
fn account_select_fetches_cache_misses_once_and_shares_the_cache() {
    let fixture = Fixture::new(&["alpha", "bravo"], "alpha");
    fixture.write_cache("alpha", 0, 60);
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, usage_body(20, 30)),
    );

    let first = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "default-with-capacity",
            "--format",
            "json",
        ],
    );
    assert_eq!(first.code, 0, "stderr: {}", first.stderr_text());
    let payload = json(&first);
    assert_eq!(payload["result"]["selected"], "bravo");
    let bravo = &payload["result"]["candidates"][1];
    assert_eq!(bravo["source"], "network");
    assert_eq!(bravo["capacity"], "available");
    assert_eq!(bravo["windows"][0]["remaining_percent"], 80);
    assert_eq!(bravo["windows"][1]["remaining_percent"], 70);
    let requests = server.take_requests();
    assert_eq!(requests.len(), 1, "only the cache miss is fetched");
    assert_eq!(
        requests[0].header_value("chatgpt-account-id").as_deref(),
        Some("acct-bravo")
    );
    assert!(cache_kv_path(&fixture.cache_root, "bravo").is_file());
    assert_no_sensitive_fields(&first);

    let second = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "default-with-capacity",
            "--format",
            "json",
        ],
    );
    assert_eq!(second.code, 0, "stderr: {}", second.stderr_text());
    assert!(
        server.take_requests().is_empty(),
        "second call reuses the cache"
    );
    assert_eq!(json(&second)["result"]["candidates"][1]["source"], "cache");
}

#[test]
fn account_select_treats_stale_cache_and_failed_fetch_as_unknown() {
    // Past the 5m TTL but inside the 600s display ceiling, so only the TTL
    // staleness rule can reject it. ALLOW_STALE must not change that.
    for allow_stale in ["false", "true"] {
        let fixture = Fixture::new(&["alpha", "bravo"], "alpha");
        fixture.write_cache("alpha", 0, 60);
        fixture.write_cache_at("bravo", Utc::now().timestamp() - 400, 50, 50);
        let server = LoopbackServer::new().expect("server");
        server.add_route("GET", "/wham/usage", HttpResponse::new(503, "{}"));

        let output = fixture.run_with_env(
            &server,
            &[
                "account",
                "select",
                "--strategy",
                "next-with-capacity",
                "--format",
                "json",
            ],
            &[("CODEX_RATE_LIMITS_CACHE_ALLOW_STALE", allow_stale)],
        );

        assert_eq!(output.code, 1, "stdout: {}", output.stdout_text());
        let payload = json(&output);
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["error"]["code"], "no-account-with-capacity");
        let bravo = &payload["error"]["details"]["candidates"][1];
        assert_eq!(bravo["capacity"], "unknown");
        assert_eq!(bravo["source"], "none");
        let requests = server.take_requests();
        assert_eq!(requests.len(), 1, "the stale entry is refetched");
        assert_eq!(
            requests[0].header_value("chatgpt-account-id").as_deref(),
            Some("acct-bravo")
        );
    }
}

#[test]
fn account_select_never_fetches_excluded_candidates() {
    let fixture = Fixture::new(&["alpha", "bravo"], "alpha");
    fixture.write_cache("alpha", 0, 60);
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, usage_body(10, 10)),
    );

    let output = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "next-with-capacity",
            "--exclude",
            "bravo",
            "--format",
            "json",
        ],
    );

    assert_eq!(output.code, 1, "stdout: {}", output.stdout_text());
    assert!(
        server.take_requests().is_empty(),
        "excluded misses are not fetched"
    );
    let payload = json(&output);
    assert_eq!(payload["error"]["code"], "no-account-with-capacity");
    let bravo = &payload["error"]["details"]["candidates"][1];
    assert_eq!(bravo["excluded"], true);
    assert_eq!(bravo["source"], "none");
}

#[test]
fn account_select_default_with_capacity_skips_fetches_when_cached_default_is_available() {
    let fixture = Fixture::new(&["alpha", "bravo", "charlie"], "bravo");
    fixture.write_cache("bravo", 40, 60);
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, usage_body(10, 10)),
    );

    let output = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "default-with-capacity",
            "--format",
            "json",
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    assert!(
        server.take_requests().is_empty(),
        "the cached default settles it"
    );
    let payload = json(&output);
    assert_eq!(payload["result"]["selected"], "bravo");
    assert_eq!(payload["result"]["candidates"][0]["source"], "none");
    assert_eq!(payload["result"]["candidates"][1]["source"], "cache");
}

#[test]
fn account_select_short_ttl_forces_network_confirmation() {
    let fixture = Fixture::new(&["alpha", "bravo"], "alpha");
    fixture.write_cache("alpha", 0, 60);
    fixture.write_cache_at("bravo", Utc::now().timestamp() - 5, 50, 50);
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, usage_body(20, 30)),
    );

    let output = fixture.run_with_env(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "next_with_capacity",
            "--format",
            "json",
        ],
        &[("CODEX_RATE_LIMITS_CACHE_TTL", "1s")],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let payload = json(&output);
    assert_eq!(payload["result"]["selected"], "bravo");
    assert_eq!(payload["result"]["candidates"][1]["source"], "network");
    // bravo's 5s-old entry must be refetched. alpha's entry may or may not
    // age past the 1s TTL before the check, so the total count is not pinned.
    let requests = server.take_requests();
    assert!(
        requests.iter().any(
            |request| request.header_value("chatgpt-account-id").as_deref() == Some("acct-bravo")
        ),
        "bravo was not refetched"
    );
}

#[test]
fn account_select_isolates_profiles_whose_cache_keys_collide() {
    // `a.b` and `a_b` normalize to the same cache key `a_b`.
    let fixture = Fixture::new(&["a.b", "a_b", "bravo"], "bravo");
    fixture.write_cache("bravo", 0, 60);
    fixture.write_cache("a_b", 0, 0);
    let shared = fs::read_to_string(cache_kv_path(&fixture.cache_root, "a_b")).unwrap();
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/wham/usage",
        HttpResponse::new(200, usage_body(20, 30)),
    );

    let output = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "next-with-capacity",
            "--format",
            "json",
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let payload = json(&output);
    assert_eq!(payload["result"]["selected"], "a.b");
    for index in [0, 1] {
        let entry = &payload["result"]["candidates"][index];
        assert_eq!(entry["source"], "network", "entry {index}: {entry}");
        assert_eq!(entry["capacity"], "available");
    }
    assert_eq!(server.take_requests().len(), 2);
    assert_eq!(
        fs::read_to_string(cache_kv_path(&fixture.cache_root, "a_b")).unwrap(),
        shared,
        "colliding profiles never write the shared entry"
    );
}

#[test]
fn account_select_reports_no_capacity_with_bounded_candidate_summary() {
    let fixture = Fixture::new(&["alpha", "bravo", "charlie"], "alpha");
    fixture.write_cache("alpha", 0, 60);
    fixture.write_cache("bravo", 40, 0);
    fixture.write_cache("charlie", 0, 0);
    let server = LoopbackServer::new().expect("server");

    for strategy in ["next-with-capacity", "default-with-capacity"] {
        let output = fixture.run(
            &server,
            &[
                "account",
                "select",
                "--strategy",
                strategy,
                "--format",
                "json",
            ],
        );
        assert_eq!(output.code, 1, "stdout: {}", output.stdout_text());
        let payload = json(&output);
        assert_eq!(payload["schema_version"], "codex-cli.account.select.v1");
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["error"]["code"], "no-account-with-capacity");
        let details = &payload["error"]["details"];
        assert_eq!(details["strategy"], strategy);
        assert_eq!(details["default_account"], "alpha");
        assert_eq!(
            candidate_names(&payload, "/error/details/candidates"),
            vec!["alpha", "bravo", "charlie"]
        );
        for entry in details["candidates"].as_array().unwrap() {
            assert_eq!(entry["capacity"], "exhausted");
        }
        assert_no_sensitive_fields(&output);
    }
    assert!(server.take_requests().is_empty());
}

#[test]
fn account_select_rejects_invalid_arguments_as_usage_errors() {
    let fixture = Fixture::new(&["alpha"], "alpha");
    let server = LoopbackServer::new().expect("server");

    let after_with_default = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "current-default",
            "--after",
            "alpha",
            "--format",
            "json",
        ],
    );
    assert_eq!(after_with_default.code, 64);
    assert_eq!(
        json(&after_with_default)["error"]["code"],
        "invalid-flag-combination"
    );

    let bad_nickname = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "next-with-capacity",
            "--exclude",
            "../alpha",
            "--format",
            "json",
        ],
    );
    assert_eq!(bad_nickname.code, 64);
    assert_eq!(json(&bad_nickname)["error"]["code"], "invalid-nickname");

    let unknown_origin = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "next-with-capacity",
            "--after",
            "zulu",
            "--format",
            "json",
        ],
    );
    assert_eq!(unknown_origin.code, 64);
    assert_eq!(json(&unknown_origin)["error"]["code"], "unknown-origin");
}

#[test]
fn account_select_without_profiles_fails_without_leaking_paths() {
    let fixture = Fixture::new(&[], "alpha");
    let server = LoopbackServer::new().expect("server");
    let output = fixture.run(
        &server,
        &[
            "account",
            "select",
            "--strategy",
            "current-default",
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 1);
    let payload = json(&output);
    assert_eq!(payload["error"]["code"], "no-account-profiles");
    assert_no_sensitive_fields(&output);
}

#[test]
fn account_select_help_documents_strategies_and_flags() {
    let group = cmd::run(&bin::resolve("codex-cli"), &["account"], &[], None);
    assert_eq!(group.code, 0);
    assert!(group.stdout_text().contains("select"));

    let help = cmd::run(
        &bin::resolve("codex-cli"),
        &["account", "select", "--help"],
        &[],
        None,
    );
    assert_eq!(help.code, 0);
    let text = help.stdout_text();
    for token in [
        "--strategy",
        "current-default",
        "next-with-capacity",
        "default-with-capacity",
        "--after",
        "--exclude",
        "--format",
    ] {
        assert!(text.contains(token), "missing {token} in help:\n{text}");
    }
}
