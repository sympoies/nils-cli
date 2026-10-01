//! `GET /usage/v1`, `POST /codex/reset/v1` (sympoies/nils-cli#1821), and
//! `POST /claude/reset/v1` (serenvia/agent-console#651).
//!
//! Both routes replace deployment-side helper services that a console edge
//! proxies to today, so the acceptance bar is response-shape compatibility with
//! what that edge already parses:
//!
//! - usage: the edge reads `data.usage.providers` (or `usage.providers`, or a
//!   bare `providers`) and keys each entry by `provider`, `account`, `label`,
//!   `ok`, `stale`, `plan`, `windows[{key,label,used_percent,window_minutes,
//!   resets_at}]`, `updated_at`, `note`, `error`, `reason_code`, and
//!   `reset_credits.available_count`. `tests/fixtures/usage-v1/edge-*.json`
//!   pin that projection.
//! - reset: the edge posts exactly `{account, idempotency_key}` and requires a
//!   top-level `schema_version` of `agent-console.codex-rate-limit-reset.v1`
//!   plus an `outcome` (and optional `windows_reset`) at the root or under
//!   `result`.
//! - Claude reset: the edge posts exactly `{account, program, idempotency_key}`
//!   and requires `agent-console.claude-limit-reset.v1` with the outcome
//!   fields at the root (`tests/fixtures/usage-v1/edge-claude-reset.json`).
//!
//! Provider CLIs are stubbed on `PATH`. Every fixture is synthetic; the stubs
//! seed `SECRET-MARKER` strings that must never reach a response.

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const TOKEN: &str = "usage-operator-token";
const MACHINE: &str = "usage-host";
const RESET_KEY: &str = "0b5f4c1e-8d2a-4c7b-9e3f-2a1b0c9d8e7f";
const OTHER_KEY: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

const USAGE_ENV: [&str; 4] = [
    "AGENT_SESSION_CODEX_RESET_ACCOUNTS",
    "AGENT_SESSION_CLAUDE_RESET_ACCOUNTS",
    "AGENT_SESSION_USAGE_V1_REFRESH_SECONDS",
    "AGENT_SESSION_MACHINE",
];

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/usage-v1")
        .join(name);
    serde_json::from_slice(&fs::read(&path).expect("fixture")).expect("fixture json")
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64
}

/// Stubbed `codex-cli` and `claude-cli`. Each run appends its argv to a log,
/// reads the configured output when it starts, optionally sleeps, and then
/// prints that output with the configured exit status, so a test can change a
/// provider's answer between requests.
struct Stubs {
    dir: PathBuf,
}

impl Stubs {
    fn new(root: &Path) -> Self {
        let dir = root.join("stub-bin");
        fs::create_dir_all(&dir).expect("stub dir");
        let stubs = Self { dir };
        for (program, kinds) in [
            (
                "codex-cli",
                &[("diag", "codex-diag"), ("account", "codex-reset")][..],
            ),
            (
                "claude-cli",
                &[("usage", "claude-usage"), ("auth", "claude-reset")][..],
            ),
        ] {
            let mut script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$1\" in\n",
                log = stubs.log_path(program).display()
            );
            for (verb, kind) in kinds {
                script.push_str(&format!(
                    "  {verb}) body=$(cat '{out}'); [ -f '{delay}' ] && sleep \"$(cat '{delay}')\"; printf '%s' \"$body\"; exit \"$(cat '{code}')\" ;;\n",
                    delay = stubs.dir.join(format!("{kind}.delay")).display(),
                    out = stubs.dir.join(format!("{kind}.out")).display(),
                    code = stubs.dir.join(format!("{kind}.exit")).display(),
                ));
            }
            script.push_str("esac\nexit 2\n");
            let path = stubs.dir.join(program);
            fs::write(&path, script).expect("stub script");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("stub mode");
        }
        stubs.answer("codex-diag", &fixture("codex-diag-all.json"), 0);
        stubs.answer(
            "claude-usage",
            &fresh_claude(fixture("claude-usage.json")),
            0,
        );
        stubs.answer("codex-reset", &fixture("codex-reset-cli.json"), 0);
        stubs.answer("claude-reset", &fixture("claude-reset-cli.json"), 0);
        stubs
    }

    fn answer(&self, kind: &str, body: &Value, exit: i32) {
        fs::write(
            self.dir.join(format!("{kind}.out")),
            serde_json::to_vec(body).expect("stub body"),
        )
        .expect("stub out");
        fs::write(self.dir.join(format!("{kind}.exit")), exit.to_string()).expect("stub exit");
    }

    fn log_path(&self, program: &str) -> PathBuf {
        self.dir.join(format!("{program}.log"))
    }

    fn calls(&self, program: &str, verb: &str) -> Vec<String> {
        fs::read_to_string(self.log_path(program))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.split(' ').next() == Some(verb))
            .map(str::to_string)
            .collect()
    }

    fn wait_for_calls(&self, program: &str, verb: &str, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.calls(program, verb).len() < count {
            assert!(
                Instant::now() < deadline,
                "{program} {verb} was not called {count} times"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// `claude-cli` reports when its snapshot was taken; windows older than the
/// staleness cap are hidden, so a stub answer must be current.
fn diag_with_alpha_credits(count: u64) -> Value {
    let mut body = fixture("codex-diag-all.json");
    body["results"][0]["reset_credits"]["available_count"] = json!(count);
    body
}

fn alpha_credits(body: &Value) -> Value {
    body["data"]["usage"]["providers"][0]["reset_credits"]["available_count"].clone()
}

fn fresh_claude(mut body: Value) -> Value {
    body["result"]["updated_at"] = json!(now_epoch());
    body
}

/// Kills its serve on drop, so a failed assertion cannot orphan a daemon.
struct Serve {
    child: Child,
    addr: SocketAddr,
}

impl Serve {
    fn spawn(root: &Path, stubs: &Stubs, env: &[(&str, &str)]) -> Self {
        let state_dir = root.join("state");
        let home = root.join("home");
        fs::create_dir_all(state_dir.join("sessions")).expect("state dir");
        fs::create_dir_all(&home).expect("home");
        for dir in [&state_dir, &state_dir.join("sessions")] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).expect("state mode");
        }
        let tmux = root.join("tmux");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf '%s\\n' 'no server running on /tmp/tmux-test/default' >&2\nexit 1\n",
        )
        .expect("fake tmux");
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).expect("tmux mode");
        let endpoint = state_dir.join("coordination/daemon-endpoint.json");
        let stderr_path = root.join("serve.stderr");
        let path = format!(
            "{}:{}",
            stubs.dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
        command
            .args(["serve", "--bind", "127.0.0.1:0", "--machine", MACHINE])
            .arg("--state-dir")
            .arg(&state_dir)
            .env("HOME", &home)
            .env("PATH", path)
            .env("AGENT_SESSION_TOKEN", TOKEN)
            .env("AGENT_SESSION_TMUX_BIN", &tmux)
            .env_remove("XDG_STATE_HOME")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(&stderr_path).expect("serve stderr"),
            ));
        for key in nils_test_support::cmd::MANAGED_SESSION_ENV
            .iter()
            .chain(USAGE_ENV.iter())
        {
            command.env_remove(key);
        }
        command.envs(env.iter().copied());
        let mut child = command.spawn().expect("spawn serve");
        let deadline = Instant::now() + Duration::from_secs(15);
        let addr = loop {
            let published = fs::read(&endpoint).ok().and_then(|raw| {
                let url = serde_json::from_slice::<Value>(&raw).ok()?["url"]
                    .as_str()?
                    .to_string();
                url.strip_prefix("http://")?.parse::<SocketAddr>().ok()
            });
            if let Some(addr) = published {
                break addr;
            }
            if let Some(status) = child.try_wait().expect("poll serve") {
                panic!(
                    "serve exited before listening: {status}; {}",
                    fs::read_to_string(&stderr_path).unwrap_or_default()
                );
            }
            assert!(
                Instant::now() < deadline,
                "serve did not publish its endpoint"
            );
            thread::sleep(Duration::from_millis(20));
        };
        Self { child, addr }
    }

    fn get(&self, path: &str, token: Option<&str>) -> (u16, Value) {
        let mut request =
            reqwest::blocking::Client::new().get(format!("http://{}{path}", self.addr));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().expect("serve request");
        let status = response.status().as_u16();
        (status, response.json::<Value>().expect("json body"))
    }

    fn post(&self, path: &str, token: Option<&str>, body: &Value) -> (u16, String) {
        self.post_with(reqwest::blocking::Client::new(), path, token, body)
            .expect("serve request")
    }

    fn post_with(
        &self,
        client: reqwest::blocking::Client,
        path: &str,
        token: Option<&str>,
        body: &Value,
    ) -> reqwest::Result<(u16, String)> {
        let mut request = client
            .post(format!("http://{}{path}", self.addr))
            .json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?;
        let status = response.status().as_u16();
        Ok((status, response.text()?))
    }

    fn usage(&self, path: &str) -> Value {
        let (status, body) = self.get(path, Some(TOKEN));
        assert_eq!(status, 200, "{body}");
        assert_no_secret(&body.to_string());
        body
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        if self.child.try_wait().expect("poll serve").is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn assert_no_secret(text: &str) {
    for marker in [
        "SECRET-MARKER",
        "example.invalid",
        "/srv/",
        "target_file",
        "cache_file",
    ] {
        assert!(!text.contains(marker), "response leaked {marker}: {text}");
    }
}

/// The providers array with each numeric `updated_at` checked for recency and
/// removed, so the rest compares against a static fixture.
fn providers_without_timestamps(body: &Value, since: i64) -> Value {
    let mut providers = body["data"]["usage"]["providers"].clone();
    for provider in providers.as_array_mut().expect("providers array") {
        let object = provider.as_object_mut().expect("provider object");
        let updated_at = object
            .remove("updated_at")
            .and_then(|value| value.as_i64())
            .expect("numeric updated_at");
        assert!(
            (since - 1..=now_epoch() + 1).contains(&updated_at),
            "updated_at {updated_at} is not current"
        );
    }
    providers
}

fn setup() -> (tempfile::TempDir, Stubs) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let stubs = Stubs::new(tmp.path());
    (tmp, stubs)
}

#[test]
fn usage_v1_requires_the_operator_bearer() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(tmp.path(), &stubs, &[]);

    let (status, body) = serve.get("/usage/v1", None);
    assert_eq!(status, 401, "{body}");
    assert_eq!(body["error"]["code"], "unauthorized");
    let (status, _) = serve.get("/usage/v1", Some("wrong-token"));
    assert_eq!(status, 401);
    assert!(stubs.calls("codex-cli", "diag").is_empty());

    let body = serve.usage("/usage/v1");
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(
        body["data"]["usage"]["schema_version"],
        "agent-session.provider-usage.v1"
    );

    let (status, body) = serve.get("/usage/v1?refresh=yes", Some(TOKEN));
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "invalid-query");
}

#[test]
fn usage_v1_projects_provider_clis_into_the_edge_contract() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(tmp.path(), &stubs, &[]);
    let since = now_epoch();

    let body = serve.usage("/usage/v1");

    assert_eq!(
        providers_without_timestamps(&body, since),
        fixture("edge-usage-providers.json")
    );
    assert_eq!(
        stubs.calls("codex-cli", "diag"),
        vec!["diag rate-limits --all --format json --no-refresh-auth"]
    );
    assert_eq!(
        stubs.calls("claude-cli", "usage"),
        vec!["usage --format json --source auto"]
    );
}

#[test]
fn usage_v1_reports_fixed_reason_codes_when_a_provider_is_unavailable() {
    let (tmp, stubs) = setup();
    stubs.answer("codex-diag", &fixture("codex-diag-failed.json"), 1);
    stubs.answer("claude-usage", &fixture("claude-usage-signed-out.json"), 0);
    let serve = Serve::spawn(tmp.path(), &stubs, &[]);

    let body = serve.usage("/usage/v1");

    assert_eq!(
        body["data"]["usage"]["providers"],
        fixture("edge-usage-providers-unavailable.json")
    );
}

#[test]
fn usage_v1_serves_the_cached_snapshot_until_a_refresh_is_forced() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(tmp.path(), &stubs, &[]);

    let first = serve.usage("/usage/v1");
    let second = serve.usage("/usage/v1");
    assert_eq!(first, second);
    assert_eq!(stubs.calls("codex-cli", "diag").len(), 1);
    assert_eq!(stubs.calls("claude-cli", "usage").len(), 1);

    stubs.answer("codex-diag", &diag_with_alpha_credits(1), 0);
    let forced = serve.usage("/usage/v1?refresh=1");
    assert_eq!(stubs.calls("codex-cli", "diag").len(), 2);
    assert_eq!(stubs.calls("claude-cli", "usage").len(), 2);
    assert_eq!(forced["data"]["usage"]["providers"][0]["stale"], false);
    assert_eq!(alpha_credits(&forced), 1);
}

#[test]
fn usage_v1_forced_refresh_waits_for_a_run_that_starts_after_the_request() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(
        tmp.path(),
        &stubs,
        &[("AGENT_SESSION_USAGE_V1_REFRESH_SECONDS", "1")],
    );
    serve.usage("/usage/v1");

    // A background refresh starts with the old answer and is still running
    // when the answer changes and a forced read arrives.
    fs::write(stubs.dir.join("codex-diag.delay"), "1").expect("delay");
    stubs.answer("codex-diag", &diag_with_alpha_credits(5), 0);
    thread::sleep(Duration::from_millis(1100));
    serve.usage("/usage/v1");
    stubs.wait_for_calls("codex-cli", "diag", 2);
    stubs.answer("codex-diag", &diag_with_alpha_credits(1), 0);

    let forced = serve.usage("/usage/v1?refresh=1");
    assert_eq!(alpha_credits(&forced), 1, "{forced}");
    assert_eq!(forced["data"]["usage"]["providers"][0]["stale"], false);
    assert_eq!(stubs.calls("codex-cli", "diag").len(), 3);
}

#[test]
fn usage_v1_serves_the_last_good_snapshot_while_a_stale_cache_refreshes() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(
        tmp.path(),
        &stubs,
        &[("AGENT_SESSION_USAGE_V1_REFRESH_SECONDS", "1")],
    );
    serve.usage("/usage/v1");

    // The next refresh is slow and then fails outright.
    fs::write(stubs.dir.join("codex-diag.delay"), "1").expect("delay");
    stubs.answer("codex-diag", &json!({}), 1);
    thread::sleep(Duration::from_millis(1100));

    let refreshing = serve.usage("/usage/v1");
    let alpha = &refreshing["data"]["usage"]["providers"][0];
    assert_eq!(alpha["account"], "alpha");
    assert_eq!(alpha["stale"], true);
    assert_eq!(alpha["windows"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        alpha["note"],
        "Refreshing Codex usage; showing the last completed result."
    );
    // An account that already failed keeps its own failure unchanged.
    assert_eq!(refreshing["data"]["usage"]["providers"][1]["stale"], false);

    stubs.wait_for_calls("codex-cli", "diag", 2);
    let deadline = Instant::now() + Duration::from_secs(10);
    let backoff = loop {
        let body = serve.usage("/usage/v1");
        let note = body["data"]["usage"]["providers"][0]["note"].clone();
        if note != "Refreshing Codex usage; showing the last completed result." {
            break body;
        }
        assert!(Instant::now() < deadline, "refresh never completed");
        thread::sleep(Duration::from_millis(50));
    };
    let alpha = &backoff["data"]["usage"]["providers"][0];
    assert_eq!(alpha["stale"], true);
    assert_eq!(alpha["windows"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        alpha["note"],
        "Codex usage refresh failed; showing the last completed result until retry."
    );
}

#[test]
fn codex_reset_is_disabled_without_an_allowlist() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(tmp.path(), &stubs, &[]);

    let (status, body) = serve.post(
        "/codex/reset/v1",
        Some(TOKEN),
        &json!({ "account": "alpha", "idempotency_key": RESET_KEY }),
    );
    assert_eq!(status, 503, "{body}");
    assert!(body.contains("codex-reset-not-configured"), "{body}");
    assert!(stubs.calls("codex-cli", "account").is_empty());
}

#[test]
fn codex_reset_rejects_unauthorized_unlisted_and_malformed_requests() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(
        tmp.path(),
        &stubs,
        &[("AGENT_SESSION_CODEX_RESET_ACCOUNTS", "alpha, charlie")],
    );
    let valid = json!({ "account": "alpha", "idempotency_key": RESET_KEY });

    let (status, _) = serve.post("/codex/reset/v1", None, &valid);
    assert_eq!(status, 401);
    let (status, _) = serve.post("/codex/reset/v1", Some("wrong-token"), &valid);
    assert_eq!(status, 401);

    for (body, expected) in [
        (json!({ "account": "alpha" }), 422),
        (
            json!({ "account": "alpha", "idempotency_key": "not-a-uuid" }),
            422,
        ),
        (
            json!({ "account": "alpha", "idempotency_key": RESET_KEY.to_uppercase() }),
            422,
        ),
        (
            json!({ "account": "alpha", "idempotency_key": RESET_KEY, "extra": true }),
            422,
        ),
        (
            json!({ "account": "../alpha", "idempotency_key": RESET_KEY }),
            422,
        ),
        (
            json!({ "account": "bravo", "idempotency_key": RESET_KEY }),
            403,
        ),
    ] {
        let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &body);
        assert_eq!(status, expected, "{body} -> {text}");
        assert_no_secret(&text);
    }
    assert!(stubs.calls("codex-cli", "account").is_empty());
}

#[test]
fn codex_reset_consumes_one_credit_and_returns_the_refreshed_usage() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(
        tmp.path(),
        &stubs,
        &[("AGENT_SESSION_CODEX_RESET_ACCOUNTS", "alpha charlie")],
    );
    serve.usage("/usage/v1");
    assert_eq!(stubs.calls("codex-cli", "diag").len(), 1);

    stubs.answer("codex-diag", &diag_with_alpha_credits(1), 0);
    let request = json!({ "account": "alpha", "idempotency_key": RESET_KEY });
    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    assert_no_secret(&text);
    let body: Value = serde_json::from_str(&text).expect("reset json");
    let expected = fixture("edge-codex-reset.json");
    for key in ["schema_version", "outcome", "windows_reset"] {
        assert_eq!(body[key], expected[key], "{key}");
    }
    assert_eq!(
        stubs.calls("codex-cli", "account"),
        vec![format!(
            "account reset-rate-limits --yes --idempotency-key {RESET_KEY} --format json alpha.json"
        )]
    );
    // The reset forces a Codex refresh and returns that snapshot.
    assert_eq!(stubs.calls("codex-cli", "diag").len(), 2);
    assert_eq!(
        body["usage"]["schema_version"],
        "agent-session.provider-usage.v1"
    );
    assert_eq!(body["usage"]["providers"][0]["account"], "alpha");
    assert_eq!(
        body["usage"]["providers"][0]["reset_credits"]["available_count"],
        1
    );
    assert_eq!(body["replayed"], false);

    // A retry with the same key replays the recorded outcome without a second
    // redemption; the same key for another account is a conflict.
    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    let replay: Value = serde_json::from_str(&text).expect("replay json");
    assert_eq!(replay["outcome"], "reset");
    assert_eq!(replay["windows_reset"], 2);
    assert_eq!(replay["replayed"], true);
    assert_eq!(stubs.calls("codex-cli", "account").len(), 1);
    // A replay serves the cached snapshot instead of forcing another refresh.
    assert_eq!(stubs.calls("codex-cli", "diag").len(), 2);

    let (status, text) = serve.post(
        "/codex/reset/v1",
        Some(TOKEN),
        &json!({ "account": "charlie", "idempotency_key": RESET_KEY }),
    );
    assert_eq!(status, 409, "{text}");
    assert!(text.contains("idempotency-key-reused"), "{text}");
    assert_eq!(stubs.calls("codex-cli", "account").len(), 1);
}

#[test]
fn codex_reset_failure_is_not_recorded_so_the_same_key_can_retry() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(
        tmp.path(),
        &stubs,
        &[("AGENT_SESSION_CODEX_RESET_ACCOUNTS", "alpha")],
    );
    stubs.answer(
        "codex-reset",
        &json!({
            "schema_version": "codex-cli.account.reset-rate-limits.v1",
            "command": "account reset-rate-limits",
            "ok": false,
            "error": { "code": "request-failed", "message": "failed for /srv/SECRET-MARKER/alpha.json" }
        }),
        1,
    );
    let request = json!({ "account": "alpha", "idempotency_key": OTHER_KEY });

    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 502, "{text}");
    assert!(text.contains("codex-reset-failed"), "{text}");
    assert_no_secret(&text);

    stubs.answer("codex-reset", &fixture("codex-reset-cli.json"), 0);
    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    assert_eq!(stubs.calls("codex-cli", "account").len(), 2);
}

fn reset_serve(tmp: &tempfile::TempDir, stubs: &Stubs) -> Serve {
    Serve::spawn(
        tmp.path(),
        stubs,
        &[("AGENT_SESSION_CODEX_RESET_ACCOUNTS", "alpha")],
    )
}

#[test]
fn concurrent_same_key_resets_redeem_once() {
    let (tmp, stubs) = setup();
    let serve = reset_serve(&tmp, &stubs);
    serve.usage("/usage/v1");
    fs::write(stubs.dir.join("codex-reset.delay"), "1").expect("delay");
    let request = json!({ "account": "alpha", "idempotency_key": RESET_KEY });

    let replays: Vec<bool> = thread::scope(|scope| {
        let posts: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
                    assert_eq!(status, 200, "{text}");
                    let body: Value = serde_json::from_str(&text).expect("reset json");
                    body["replayed"].as_bool().expect("replayed flag")
                })
            })
            .collect();
        posts
            .into_iter()
            .map(|post| post.join().expect("post"))
            .collect()
    });

    assert_eq!(replays.iter().filter(|replayed| **replayed).count(), 1);
    assert_eq!(stubs.calls("codex-cli", "account").len(), 1);
}

#[test]
fn a_disconnected_reset_still_finishes_and_is_replayed() {
    let (tmp, stubs) = setup();
    let serve = reset_serve(&tmp, &stubs);
    serve.usage("/usage/v1");
    fs::write(stubs.dir.join("codex-reset.delay"), "2").expect("delay");
    stubs.answer("codex-diag", &diag_with_alpha_credits(1), 0);
    let request = json!({ "account": "alpha", "idempotency_key": RESET_KEY });

    let impatient = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .expect("client");
    assert!(
        serve
            .post_with(impatient, "/codex/reset/v1", Some(TOKEN), &request)
            .is_err(),
        "the first request should time out while the reset runs"
    );

    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    let body: Value = serde_json::from_str(&text).expect("reset json");
    assert_eq!(body["replayed"], true);
    assert_eq!(stubs.calls("codex-cli", "account").len(), 1);
    // The recorded reset invalidated the cache, so the replay never serves the
    // pre-reset numbers as fresh.
    let alpha = &body["usage"]["providers"][0];
    assert!(
        alpha["stale"] == true || alpha["reset_credits"]["available_count"] == 1,
        "{alpha}"
    );
}

#[test]
fn a_reset_answers_within_its_budget_when_the_refresh_is_slow() {
    let (tmp, stubs) = setup();
    let serve = reset_serve(&tmp, &stubs);
    serve.usage("/usage/v1");
    fs::write(stubs.dir.join("codex-diag.delay"), "8").expect("delay");
    let request = json!({ "account": "alpha", "idempotency_key": RESET_KEY });

    let started = Instant::now();
    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
    let elapsed = started.elapsed();

    assert_eq!(status, 200, "{text}");
    assert!(
        elapsed < Duration::from_millis(7500),
        "reset took {elapsed:?}"
    );
    let body: Value = serde_json::from_str(&text).expect("reset json");
    assert_eq!(body["outcome"], "reset");
    let alpha = &body["usage"]["providers"][0];
    assert_eq!(alpha["stale"], true);
    assert_eq!(
        alpha["note"],
        "Refreshing Codex usage; showing the last completed result."
    );
}

fn claude_reset_serve(tmp: &tempfile::TempDir, stubs: &Stubs) -> Serve {
    Serve::spawn(
        tmp.path(),
        stubs,
        &[("AGENT_SESSION_CLAUDE_RESET_ACCOUNTS", "alpha, charlie")],
    )
}

fn claude_request(account: &str, program: &str, key: &str) -> Value {
    json!({ "account": account, "program": program, "idempotency_key": key })
}

#[test]
fn claude_reset_is_disabled_without_an_allowlist() {
    let (tmp, stubs) = setup();
    let serve = Serve::spawn(
        tmp.path(),
        &stubs,
        &[("AGENT_SESSION_CODEX_RESET_ACCOUNTS", "alpha")],
    );

    let (status, body) = serve.post(
        "/claude/reset/v1",
        Some(TOKEN),
        &claude_request("alpha", "cedar_ember", RESET_KEY),
    );
    assert_eq!(status, 503, "{body}");
    assert!(body.contains("claude-reset-not-configured"), "{body}");
    assert!(stubs.calls("claude-cli", "auth").is_empty());
}

#[test]
fn claude_reset_rejects_unauthorized_unlisted_and_malformed_requests() {
    let (tmp, stubs) = setup();
    let serve = claude_reset_serve(&tmp, &stubs);
    let valid = claude_request("alpha", "juniper_tide", RESET_KEY);

    let (status, _) = serve.post("/claude/reset/v1", None, &valid);
    assert_eq!(status, 401);
    let (status, _) = serve.post("/claude/reset/v1", Some("wrong-token"), &valid);
    assert_eq!(status, 401);

    for (body, expected) in [
        (
            json!({ "account": "alpha", "idempotency_key": RESET_KEY }),
            422,
        ),
        (claude_request("alpha", "free_lunch", RESET_KEY), 422),
        (claude_request("alpha", "JUNIPER_TIDE", RESET_KEY), 422),
        (claude_request("alpha", "juniper_tide", "not-a-uuid"), 422),
        (
            claude_request("alpha", "juniper_tide", &RESET_KEY.to_uppercase()),
            422,
        ),
        (claude_request("../alpha", "juniper_tide", RESET_KEY), 422),
        (
            json!({ "account": "alpha", "program": "juniper_tide", "idempotency_key": RESET_KEY, "extra": 1 }),
            422,
        ),
        (
            json!({ "account": "alpha", "program": 1, "idempotency_key": RESET_KEY }),
            422,
        ),
        (claude_request("bravo", "juniper_tide", RESET_KEY), 403),
    ] {
        let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &body);
        assert_eq!(status, expected, "{body} -> {text}");
        assert_no_secret(&text);
    }
    assert!(stubs.calls("claude-cli", "auth").is_empty());
}

#[test]
fn claude_reset_redeems_once_and_replays_the_recorded_outcome() {
    let (tmp, stubs) = setup();
    let serve = claude_reset_serve(&tmp, &stubs);
    serve.usage("/usage/v1");
    assert_eq!(stubs.calls("claude-cli", "usage").len(), 1);
    let request = claude_request("alpha", "cedar_ember", RESET_KEY);

    let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    assert_no_secret(&text);
    let body: Value = serde_json::from_str(&text).expect("reset json");
    assert_eq!(body, fixture("edge-claude-reset.json"));
    assert_eq!(
        stubs.calls("claude-cli", "auth"),
        vec![format!(
            "auth reset-rate-limits --yes --program cedar_ember --request-id {RESET_KEY} --format json alpha"
        )]
    );
    // A recorded reset refreshes the Claude usage slot.
    stubs.wait_for_calls("claude-cli", "usage", 2);

    // Same key, account, and program: replay without a second redemption.
    let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    let replay: Value = serde_json::from_str(&text).expect("replay json");
    let mut expected = fixture("edge-claude-reset.json");
    expected["replayed"] = json!(true);
    assert_eq!(replay, expected);
    assert_eq!(stubs.calls("claude-cli", "auth").len(), 1);

    // The same key for another program or account is a conflict.
    for other in [
        claude_request("alpha", "juniper_tide", RESET_KEY),
        claude_request("charlie", "cedar_ember", RESET_KEY),
    ] {
        let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &other);
        assert_eq!(status, 409, "{text}");
        assert!(text.contains("idempotency-key-reused"), "{text}");
    }
    assert_eq!(stubs.calls("claude-cli", "auth").len(), 1);

    // Codex and Claude keep separate replay records.
    let (status, text) = serve.post(
        "/codex/reset/v1",
        Some(TOKEN),
        &json!({ "account": "alpha", "idempotency_key": RESET_KEY }),
    );
    assert_eq!(status, 503, "{text}");
}

#[test]
fn claude_reset_reports_an_unavailable_program_without_posting() {
    let (tmp, stubs) = setup();
    let serve = claude_reset_serve(&tmp, &stubs);
    stubs.answer(
        "claude-reset",
        &json!({
            "schema_version": "claude-cli.auth.reset-rate-limits.v1",
            "command": "auth reset-rate-limits",
            "ok": true,
            "result": {
                "provider": "claude", "program": "juniper_tide", "outcome": "unavailable",
                "posted": false, "reason": "not_at_wall", "resets_left": null,
                "next_available_at": null, "cooldown_until": null, "weekly_resets_at": null
            }
        }),
        0,
    );

    let (status, text) = serve.post(
        "/claude/reset/v1",
        Some(TOKEN),
        &claude_request("charlie", "juniper_tide", OTHER_KEY),
    );
    assert_eq!(status, 200, "{text}");
    let body: Value = serde_json::from_str(&text).expect("reset json");
    assert_eq!(
        body,
        json!({
            "schema_version": "agent-console.claude-limit-reset.v1",
            "program": "juniper_tide",
            "outcome": "unavailable",
            "posted": false,
            "reason": "not_at_wall",
            "resets_left": null,
            "next_available_at": null,
            "cooldown_until": null,
            "weekly_resets_at": null,
            "replayed": false,
            "machine": MACHINE
        })
    );
}

#[test]
fn claude_reset_failure_surfaces_a_safe_cli_code_and_is_not_recorded() {
    let (tmp, stubs) = setup();
    let serve = claude_reset_serve(&tmp, &stubs);
    let request = claude_request("alpha", "cedar_ember", OTHER_KEY);

    for (cli_code, reason, retryable, expected_code) in [
        (
            "provider-unavailable",
            Some("rate_limited"),
            true,
            Some("provider-unavailable"),
        ),
        (
            "claude-auth-required",
            Some("auth_expired"),
            false,
            Some("claude-auth-required"),
        ),
        ("SECRET-MARKER-code", None, false, None),
    ] {
        stubs.answer(
            "claude-reset",
            &json!({
                "schema_version": "claude-cli.auth.reset-rate-limits.v1",
                "command": "auth reset-rate-limits",
                "ok": false,
                "error": {
                    "code": cli_code,
                    "message": "failed for /srv/SECRET-MARKER/alpha.json",
                    "details": { "retryable": retryable, "reason_code": reason }
                }
            }),
            3,
        );
        let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &request);
        assert_eq!(status, 502, "{text}");
        assert_no_secret(&text);
        let body: Value = serde_json::from_str(&text).expect("error json");
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "claude-reset-failed");
        match expected_code {
            Some(code) => assert_eq!(
                body["error"]["details"],
                json!({ "cli_code": code, "reason_code": reason, "retryable": retryable })
            ),
            None => assert!(body["error"].get("details").is_none(), "{body}"),
        }
    }

    for invalid in [
        json!({ "ok": true, "result": { "outcome": "reset" } }),
        {
            let mut body = fixture("claude-reset-cli.json");
            body["result"]["outcome"] = json!("consumed");
            body
        },
        {
            let mut body = fixture("claude-reset-cli.json");
            body["result"]["program"] = json!("juniper_tide");
            body
        },
        {
            let mut body = fixture("claude-reset-cli.json");
            body["result"]["reason"] = json!("Free Text SECRET-MARKER");
            body
        },
    ] {
        stubs.answer("claude-reset", &invalid, 0);
        let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &request);
        assert_eq!(status, 502, "{text}");
        assert!(text.contains("claude-reset-invalid-response"), "{text}");
        assert_no_secret(&text);
    }

    // Nothing failed was recorded, so the same key still runs.
    stubs.answer("claude-reset", &fixture("claude-reset-cli.json"), 0);
    let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    assert_eq!(stubs.calls("claude-cli", "auth").len(), 8);
}

#[test]
fn a_disconnected_claude_reset_still_finishes_and_is_replayed() {
    let (tmp, stubs) = setup();
    let serve = claude_reset_serve(&tmp, &stubs);
    fs::write(stubs.dir.join("claude-reset.delay"), "2").expect("delay");
    let request = claude_request("alpha", "cedar_ember", RESET_KEY);

    let impatient = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .expect("client");
    assert!(
        serve
            .post_with(impatient, "/claude/reset/v1", Some(TOKEN), &request)
            .is_err(),
        "the first request should time out while the reset runs"
    );

    let (status, text) = serve.post("/claude/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    let body: Value = serde_json::from_str(&text).expect("reset json");
    assert_eq!(body["replayed"], true);
    assert_eq!(body["outcome"], "reset");
    assert_eq!(stubs.calls("claude-cli", "auth").len(), 1);
}
