use crate::support::*;
use nils_test_support::http::{HttpResponse, TestServer};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

const BODY: &str = r#"{"five_hour":{"utilization":25,"resets_at":"2030-01-01T00:00:00Z"},"seven_day":{"utilization":40,"resets_at":"2030-01-02T00:00:00Z"}}"#;

fn state_file(root: &Path) -> std::path::PathBuf {
    std::fs::read_dir(root.join("usage-backoff"))
        .expect("persistent backoff directory")
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .expect("persistent backoff state")
}

#[test]
fn usage_backoff_zero_retry_after_records_five_minutes() {
    let tmp = tempfile::tempdir().unwrap();
    let server =
        TestServer::new(|_| HttpResponse::new(429, "rate limited").with_header("Retry-After", "0"))
            .unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_exit(
        &run(
            &["usage", "--source", "oauth", "--format", "json"],
            &options,
        ),
        0,
    );
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(state_file(tmp.path())).unwrap()).unwrap();
    assert!(state["retry_at"].as_u64().unwrap() >= before + 300);
    assert!(
        !std::fs::read_to_string(state_file(tmp.path()))
            .unwrap()
            .contains("fixture-token")
    );
}

#[test]
fn usage_backoff_second_caller_skips_http_and_preserves_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let limited = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&limited);
    let server = TestServer::new(move |_| {
        if flag.load(Ordering::SeqCst) {
            HttpResponse::new(429, "rate limited")
        } else {
            HttpResponse::new(200, BODY)
        }
    })
    .unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    assert_exit(&run(&["usage", "--source", "oauth"], &options), 0);
    assert_eq!(server.take_requests().len(), 1);
    limited.store(true, Ordering::SeqCst);
    assert_exit(&run(&["prompt-segment", "--refresh"], &options), 0);
    assert_eq!(server.take_requests().len(), 1);
    let output = run(&["usage", "--format", "json"], &options);
    assert_exit(&output, 0);
    assert_eq!(
        server.take_requests().len(),
        0,
        "a different caller must share the cooldown"
    );
    let result: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(result["result"]["reason_code"], "rate_limited");
    assert_eq!(result["result"]["stale"], true);
    assert_eq!(result["result"]["windows"][0]["remaining_percent"], 75.0);
    let prompt = run(&["prompt-segment"], &options);
    assert_exit(&prompt, 0);
    assert!(stdout(&prompt).contains("(stale)"));
    assert!(server.take_requests().is_empty());
}

#[test]
fn usage_backoff_oauth_429_never_launches_cli_probe() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("probe-called");
    let bin_dir = write_fake_claude(
        tmp.path(),
        "#!/usr/bin/env sh\nprintf called > \"$PROBE_MARKER\"\ncat >/dev/null\n",
    );
    let server = TestServer::new(|_| HttpResponse::new(429, "rate limited")).unwrap();
    let options = base_options(tmp.path())
        .with_fake_claude(&bin_dir)
        .with_env("PROBE_MARKER", &path_str(&marker))
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-token")
        .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_STARTUP_DELAY_MS", "1")
        .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_USAGE_DELAY_MS", "1")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    assert_exit(&run(&["usage", "--format", "json"], &options), 0);
    assert!(
        !marker.exists(),
        "OAuth 429 must not launch the PTY or pipe probe"
    );
    assert_eq!(server.take_requests().len(), 1);
}

#[test]
fn usage_backoff_success_clears_expired_state() {
    let tmp = tempfile::tempdir().unwrap();
    let limited = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&limited);
    let server = TestServer::new(move |_| {
        if flag.load(Ordering::SeqCst) {
            HttpResponse::new(429, "rate limited")
        } else {
            HttpResponse::new(200, BODY)
        }
    })
    .unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    let args = ["usage", "--source", "oauth", "--format", "json"];
    assert_exit(&run(&args, &options), 0);
    let path = state_file(tmp.path());
    // Advance eligibility without sleeping or changing the machine clock.
    std::fs::write(&path, json!({"retry_at":0,"delay_seconds":300}).to_string()).unwrap();
    limited.store(false, Ordering::SeqCst);
    let output = run(&args, &options);
    assert_exit(&output, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["result"]["source"],
        "oauth"
    );
    assert!(
        !path.exists(),
        "success must clear consecutive rate-limit state"
    );
    assert_exit(&run(&args, &options), 0);
    assert_eq!(server.take_requests().len(), 3);
}

#[test]
fn usage_backoff_shared_diag_modes_preserve_each_account_and_block_reset_status_read() {
    let tmp = tempfile::tempdir().unwrap();
    let limited = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&limited);
    let server = TestServer::new(move |request| {
        if flag.load(Ordering::SeqCst)
            && request.header_value("authorization").as_deref() == Some("Bearer fixture-alpha")
        {
            HttpResponse::new(429, "rate limited")
        } else {
            HttpResponse::new(200, BODY)
        }
    })
    .unwrap();
    let profiles = tmp.path().join("profiles");
    std::fs::create_dir(&profiles).unwrap();
    for name in ["alpha", "beta"] {
        std::fs::write(profiles.join(format!("{name}.json")), json!({
            "claudeAiOauth": {"accessToken": format!("fixture-{name}"), "expiresAt": 4102444800000i64},
            "oauthAccount": {"accountUuid": format!("fixture-{name}"), "organizationUuid": "fixture-org"}
        }).to_string()).unwrap();
    }
    let options = base_options(tmp.path())
        .with_env("CLAUDE_SECRET_DIR", &path_str(&profiles))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-alpha")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    assert_exit(
        &run(&["diag", "rate-limits", "--all", "--json"], &options),
        0,
    );
    assert_eq!(server.take_requests().len(), 2);
    limited.store(true, Ordering::SeqCst);
    assert_exit(
        &run(
            &["usage", "--source", "oauth", "--format", "json"],
            &options,
        ),
        0,
    );
    assert_eq!(server.take_requests().len(), 1);
    for mode in ["--all", "--async"] {
        let output = run(&["diag", "rate-limits", mode, "--json"], &options);
        assert_eq!(output.code, 0, "{}", stdout(&output));
        let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
        let results = payload["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        let alpha = results
            .iter()
            .find(|value| value["name"] == "alpha")
            .unwrap();
        assert_eq!(alpha["source"], "cache-fallback");
        assert_eq!(alpha["reason_code"], "rate_limited");
        assert_eq!(alpha["windows"][0]["remaining_percent"], 75);
        let requests = server.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].header_value("authorization").as_deref(),
            Some("Bearer fixture-beta")
        );
    }
    let output = run(
        &["diag", "rate-limits", "--async", "--json", "--cached"],
        &options,
    );
    assert_eq!(output.code, 0, "{}", stdout(&output));
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["results"][0]["reason_code"],
        "rate_limited"
    );
    let output = run(
        &[
            "auth",
            "reset-rate-limits",
            "--program",
            "juniper_tide",
            "--yes",
            "--request-id",
            "0b5f4c1e-8d2a-4c7b-9e3f-2a1b0c9d8e7f",
            "--format",
            "json",
            "alpha",
        ],
        &options,
    );
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["error"]["details"]["reason_code"],
        "rate_limited"
    );
    assert!(
        server.take_requests().is_empty(),
        "cached diagnostics and reset status must not request during backoff"
    );
}

#[test]
fn usage_backoff_concurrent_processes_make_one_request_for_the_same_token() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::new(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        HttpResponse::new(429, "rate limited")
    })
    .unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let options = &options;
            scope.spawn(move || assert_exit(&run(&["usage", "--source", "oauth"], options), 0));
        }
    });
    assert_eq!(server.take_requests().len(), 1);
}

#[test]
fn usage_backoff_403_rate_limit_and_positive_retry_after_are_shared() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::new(|_| {
        HttpResponse::new(403, "too many requests").with_header("Retry-After", "900")
    })
    .unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let output = run(
        &["usage", "--source", "oauth", "--format", "json"],
        &options,
    );
    assert_exit(&output, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["result"]["reason_code"],
        "rate_limited"
    );
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(state_file(tmp.path())).unwrap()).unwrap();
    assert!(state["retry_at"].as_u64().unwrap() >= before + 900);
    assert_exit(&run(&["prompt-segment", "--refresh"], &options), 0);
    assert_eq!(server.take_requests().len(), 1);
}

#[test]
fn usage_backoff_fresh_prompt_never_resolves_keychain_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    write_cache(tmp.path(), BODY);
    let bin = tmp.path().join("fake-bin");
    std::fs::create_dir(&bin).unwrap();
    let marker = tmp.path().join("keychain-called");
    let script = bin.join("security");
    std::fs::write(
        &script,
        "#!/usr/bin/env sh\nprintf called > \"$KEYCHAIN_MARKER\"\nexit 1\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let options = base_options(tmp.path())
        .with_path_prepend(&bin)
        .with_env("CLAUDE_PROMPT_SEGMENT_KEYCHAIN_DISABLED", "0")
        .with_env("KEYCHAIN_MARKER", &path_str(&marker));
    let output = run(&["prompt-segment"], &options);
    assert_exit(&output, 0);
    assert!(!stdout(&output).is_empty());
    assert!(
        !marker.exists(),
        "fresh cached prompt must render without a Keychain process"
    );
}

#[test]
fn usage_backoff_unavailable_cache_keeps_successful_live_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let blocked = tmp.path().join("cache-is-a-file");
    std::fs::write(&blocked, "fixture").unwrap();
    let server = TestServer::new(|_| HttpResponse::new(200, BODY)).unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_CACHE_DIR", &path_str(&blocked))
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    let output = run(
        &["usage", "--source", "oauth", "--format", "json"],
        &options,
    );
    assert_exit(&output, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["result"]["source"],
        "oauth"
    );
    assert_eq!(server.take_requests().len(), 1);
}

#[test]
fn usage_backoff_never_returns_another_tokens_cached_windows() {
    let tmp = tempfile::tempdir().unwrap();
    let server = TestServer::new(|request| {
        if request.header_value("authorization").as_deref() == Some("Bearer fixture-alpha") {
            HttpResponse::new(200, BODY)
        } else {
            HttpResponse::new(429, "rate limited")
        }
    })
    .unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-alpha")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    let args = ["usage", "--source", "oauth", "--format", "json"];
    assert_exit(&run(&args, &options), 0);
    let limited = options.with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-beta");
    for source in ["oauth", "auto"] {
        let output = run(&["usage", "--source", source, "--format", "json"], &limited);
        assert_exit(&output, 0);
        let payload: Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(payload["result"]["reason_code"], "rate_limited");
        assert_eq!(
            payload["result"]["windows"],
            json!([]),
            "limited beta must not receive alpha's windows"
        );
    }
    let prompt = run(&["prompt-segment", "--refresh"], &limited);
    assert_exit(&prompt, 0);
    assert!(
        stdout(&prompt).is_empty(),
        "limited beta must not render alpha's prompt cache"
    );
    assert_eq!(server.take_requests().len(), 2);
}
