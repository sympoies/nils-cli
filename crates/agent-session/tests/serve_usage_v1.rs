//! `GET /usage/v1` and `POST /codex/reset/v1` (sympoies/nils-cli#1821).
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

const USAGE_ENV: [&str; 3] = [
    "AGENT_SESSION_CODEX_RESET_ACCOUNTS",
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

/// Stubbed `codex-cli` and `claude-cli`. Each run appends its argv to a log and
/// prints the configured output with the configured exit status, so a test can
/// change a provider's answer between requests.
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
            ("claude-cli", &[("usage", "claude-usage")][..]),
        ] {
            let mut script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\ncase \"$1\" in\n",
                log = stubs.log_path(program).display()
            );
            for (verb, kind) in kinds {
                script.push_str(&format!(
                    "  {verb}) [ -f '{delay}' ] && sleep \"$(cat '{delay}')\"; cat '{out}'; exit \"$(cat '{code}')\" ;;\n",
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
        let mut request = reqwest::blocking::Client::new()
            .post(format!("http://{}{path}", self.addr))
            .json(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().expect("serve request");
        let status = response.status().as_u16();
        (status, response.text().expect("body text"))
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

    let forced = serve.usage("/usage/v1?refresh=1");
    assert_eq!(stubs.calls("codex-cli", "diag").len(), 2);
    assert_eq!(stubs.calls("claude-cli", "usage").len(), 2);
    assert_eq!(forced["data"]["usage"]["providers"][0]["stale"], false);
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

    // A retry with the same key replays the recorded outcome without a second
    // redemption; the same key for another account is a conflict.
    let (status, text) = serve.post("/codex/reset/v1", Some(TOKEN), &request);
    assert_eq!(status, 200, "{text}");
    let replay: Value = serde_json::from_str(&text).expect("replay json");
    assert_eq!(replay["outcome"], "reset");
    assert_eq!(replay["windows_reset"], 2);
    assert_eq!(stubs.calls("codex-cli", "account").len(), 1);

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
