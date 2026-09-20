use crate::support::*;
use nils_test_support::http::{HttpResponse, LoopbackServer, TestServer};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[test]
fn prompt_segment_check_reads_credentials_json_without_printing_token() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let token = "secret-token-for-check";
    let options = base_options(tmp.path()).with_env(
        "CLAUDE_PROMPT_SEGMENT_CREDENTIALS_JSON",
        &format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}"}}}}"#),
    );

    let output = run(&["prompt-segment", "check"], &options);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

#[test]
fn prompt_segment_is_enabled_returns_one_when_credentials_are_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = run(
        &["prompt-segment", "--is-enabled"],
        &base_options(tmp.path()),
    );
    assert_exit(&output, 1);
    assert_eq!(stdout(&output), "");
}

#[test]
fn prompt_segment_renders_fresh_cached_usage_without_credentials() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_cache(tmp.path(), &usage_json(23.2, 44.1));

    let options = base_options(tmp.path())
        .with_env("NO_COLOR", "1")
        .with_env("TZ", "UTC");
    let output = run(
        &["prompt-segment", "--ttl", "1h", "--time-format", "%Y-%m-%d"],
        &options,
    );

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:77% W:56% 2026-01-03\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn prompt_segment_refresh_fetches_and_writes_cache_without_secret_leakage() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = LoopbackServer::new().expect("server");
    let body = usage_json(25.0, 50.0);
    server.add_route(
        "GET",
        "/usage",
        HttpResponse::new(200, body.clone()).with_header("Content-Type", "application/json"),
    );

    let token = "secret-token-refresh";
    let endpoint = format!("{}/usage", server.url());
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", token)
        .with_env("CLAUDE_PROMPT_SEGMENT_ENDPOINT", &endpoint)
        .with_env("NO_COLOR", "1")
        .with_env("TZ", "UTC");

    let output = run(
        &["prompt-segment", "--refresh", "--time-format", "%Y-%m-%d"],
        &options,
    );

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:75% W:50% 2026-01-03\n");
    assert!(!stdout(&output).contains(token));
    assert!(!stderr(&output).contains(token));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("usage.json")).expect("cache"),
        body
    );

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].header_value("authorization"),
        Some(format!("Bearer {token}"))
    );
    assert_eq!(
        requests[0].header_value("anthropic-beta"),
        Some("oauth-2025-04-20".to_string())
    );
    assert_eq!(
        requests[0].header_value("user-agent"),
        Some("claude-code/2.1.0".to_string())
    );
}

#[test]
fn prompt_segment_stale_cache_fallback_suppresses_fetch_errors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    make_old(&cache_file);

    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/usage", HttpResponse::new(500, "upstream exploded"));

    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "secret-token-stale")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        )
        .with_env("NO_COLOR", "1")
        .with_env("TZ", "UTC");

    let output = run(
        &["prompt-segment", "--ttl", "1s", "--time-format", "%Y-%m-%d"],
        &options,
    );

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "5h:75% W:50% 2026-01-03 (stale)\n");
    assert_eq!(stderr(&output), "");
    // The stale render enqueues a detached refresh whose fetch fails. Its
    // `.refresh.at` write lands after this test body returns unless we wait, and
    // `write_atomic` re-creates `tmp` on the way.
    assert_eq!(wait_for_requests(&server, 1).len(), 1);
    assert!(
        wait_for_refresh_lock_release(&cache_file, Duration::from_secs(4)),
        "detached refresh still held the lock at teardown"
    );
}

#[test]
fn prompt_segment_does_not_render_cache_older_than_max_stale_age() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    set_modified(&cache_file, SystemTime::now() - Duration::from_secs(601));
    // The second uncontained spawner, which the first pass classified wrongly: a
    // 601s-old cache is display-expired, so this run spawns a detached child that
    // can recreate `tmp` after teardown. Its subject is max-stale rendering, so
    // hold the cooldown.
    //
    // This one is the falsifiable case: disabling the gate makes the run spawn and
    // `assert_held` fails with its own message, so the containment is demonstrated
    // rather than assumed.
    let cooldown = RefreshCooldown::hold(tmp.path());

    let output = run(
        &["prompt-segment", "--ttl", "1h", "--time-format", "%Y-%m-%d"],
        &base_options(tmp.path())
            .with_env("NO_COLOR", "1")
            .with_env(cooldown.env().0, cooldown.env().1),
    );

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "");
    assert!(
        cache_file.is_file(),
        "max-stale handling must not delete cache"
    );
    cooldown.assert_held();
}

#[test]
fn prompt_segment_expired_cache_failed_refresh_observes_cooldown() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    set_modified(&cache_file, SystemTime::now() - Duration::from_secs(601));

    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/usage", HttpResponse::new(500, "upstream exploded"));
    let options = base_options(tmp.path())
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN",
            "secret-token-cooldown",
        )
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );

    for _ in 0..2 {
        let output = run(&["prompt-segment", "--ttl", "1h"], &options);
        assert_exit(&output, 0);
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output), "");
    }

    assert_eq!(wait_for_requests(&server, 1).len(), 1);
    // The observed request proves the detached child holds the refresh lock; it
    // still writes the `.refresh.at` marker before releasing, which would
    // re-create `tmp` after teardown.
    assert!(
        wait_for_refresh_lock_release(&cache_file, Duration::from_secs(4)),
        "detached refresh still held the lock at teardown"
    );
}

#[test]
fn prompt_segment_expired_cache_concurrent_refreshes_are_coalesced() {
    const WORKERS: usize = 6;

    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    set_modified(&cache_file, SystemTime::now() - Duration::from_secs(601));

    let server = TestServer::new(|_| {
        thread::sleep(Duration::from_millis(400));
        HttpResponse::new(500, "upstream exploded")
    })
    .expect("server");
    let shim_dir = tmp.path().join("refresh-shim");
    std::fs::create_dir_all(&shim_dir).expect("shim dir");
    let shim = shim_dir.join("claude-cli-refresh");
    let launch_log = tmp.path().join("refresh-launches.log");
    nils_test_support::write_exe(
        &shim_dir,
        "claude-cli-refresh",
        "#!/bin/sh\nprintf '%s\\n' launch >> \"$CLAUDE_TEST_REFRESH_LAUNCH_LOG\"\nexec \"$CLAUDE_TEST_REAL_EXE\" \"$@\"\n",
    );
    let options = base_options(tmp.path())
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN",
            "secret-token-coalesced",
        )
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        )
        .with_env("CLAUDE_PROMPT_SEGMENT_EXE", &path_str(&shim))
        .with_env("CLAUDE_TEST_REAL_EXE", &path_str(&claude_cli_bin()))
        .with_env("CLAUDE_TEST_REFRESH_LAUNCH_LOG", &path_str(&launch_log));
    let barrier = Arc::new(Barrier::new(WORKERS));

    let handles = (0..WORKERS)
        .map(|_| {
            let options = options.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                run(&["prompt-segment", "--ttl", "1h"], &options)
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        let output = handle.join().expect("prompt worker");
        assert_exit(&output, 0);
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output), "");
    }

    assert_eq!(wait_for_test_requests(&server, 1).len(), 1);
    assert_eq!(
        std::fs::read_to_string(launch_log)
            .expect("refresh launch log")
            .lines()
            .count(),
        1
    );
    assert!(
        wait_for_refresh_lock_release(&cache_file, Duration::from_secs(4)),
        "coalesced refresh still held the lock at teardown"
    );
}

#[test]
fn prompt_segment_explicit_refresh_bypasses_expired_cache_cooldown() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    set_modified(&cache_file, SystemTime::now() - Duration::from_secs(601));

    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/usage", HttpResponse::new(500, "upstream exploded"));
    let options = base_options(tmp.path())
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN",
            "secret-token-explicit",
        )
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );

    for _ in 0..2 {
        let output = run(&["prompt-segment", "--refresh"], &options);
        assert_exit(&output, 0);
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output), "");
    }

    assert_eq!(server.take_requests().len(), 2);
}

#[test]
fn prompt_segment_status_reports_expired_cache_without_rendering() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    set_modified(&cache_file, SystemTime::now() - Duration::from_secs(601));

    let output = run(
        &["prompt-segment", "status", "--format", "json"],
        &base_options(tmp.path())
            .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "secret-token-status"),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["cache_exists"], true);
    assert_eq!(payload["result"]["cache_stale"], true);
    assert_eq!(payload["result"]["would_render"], false);
    assert_eq!(payload["result"]["reason"], "cache-expired");
    assert!(cache_file.is_file(), "status must not delete expired cache");
}

#[test]
fn prompt_segment_missing_credentials_and_cache_is_quiet_success() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Per-test enumeration (#1412) showed this test reaches
    // `enqueue_background_refresh` despite having neither credentials nor a
    // cache, with no containment at all. Its subject is the quiet-success
    // rendering, not the refresh, so hold the cooldown.
    //
    // Measured: with the cooldown held this run enters `enqueue_background_refresh`
    // and spawns nothing. Unlike the max-stale case below, disabling the gate does
    // not make it spawn either — holding the cooldown also creates the cache
    // directory, which changes the rest of the path — so `assert_held` here is a
    // tripwire that cannot currently fire rather than a proof. Kept because it
    // costs nothing and would catch the marker being rewritten if that changes.
    let cooldown = RefreshCooldown::hold(tmp.path());
    let options = base_options(tmp.path()).with_env(cooldown.env().0, cooldown.env().1);

    let output = run(&["prompt-segment"], &options);

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
    cooldown.assert_held();
}

#[test]
fn prompt_segment_status_json_has_stable_envelope_and_no_secret_leakage() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let token = "secret-token-status";
    let options = base_options(tmp.path()).with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", token);

    let output = run(&["prompt-segment", "status", "--format", "json"], &options);
    assert_exit(&output, 0);
    assert!(!stdout(&output).contains(token));
    assert!(!stderr(&output).contains(token));

    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "claude-cli.prompt-segment.v1");
    assert_eq!(payload["command"], "prompt-segment status");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["authenticated"], true);
    assert_eq!(payload["result"]["auth_source"], "access-token-env");
    assert_eq!(payload["result"]["cache_exists"], false);
    assert_eq!(payload["result"]["reason"], "cache-missing");
}

#[test]
fn prompt_segment_render_flags_filter_window_show_timezone_and_escape_zsh_percent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_cache(tmp.path(), &usage_json(25.0, 50.0));
    let options = base_options(tmp.path())
        .with_env("NO_COLOR", "1")
        .with_env("TZ", "UTC")
        .with_env("CLAUDE_PROMPT_SEGMENT_ZSH_ESCAPE_ENABLED", "1");

    let output = run(&["prompt-segment", "--no-5h", "--show-timezone"], &options);

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "W:50%% 01-03 12:30 +00:00\n");
}

#[test]
fn prompt_segment_stale_cache_returns_immediately_and_refreshes_in_background() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    make_old(&cache_file);
    let refreshed_body = usage_json(10.0, 20.0);
    let response_body = refreshed_body.clone();
    let server = TestServer::new(move |_| {
        thread::sleep(Duration::from_millis(800));
        HttpResponse::new(200, response_body.clone())
            .with_header("Content-Type", "application/json")
    })
    .expect("server");
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "background-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        )
        .with_env("NO_COLOR", "1")
        .with_env("TZ", "UTC");

    let started = Instant::now();
    let output = run(
        &["prompt-segment", "--ttl", "1s", "--time-format", "%Y-%m-%d"],
        &options,
    );
    let elapsed = started.elapsed();

    assert_exit(&output, 0);
    assert!(
        elapsed < Duration::from_millis(400),
        "cached prompt path blocked for {elapsed:?}"
    );
    assert_eq!(stdout(&output), "5h:75% W:50% 2026-01-03 (stale)\n");

    assert!(
        wait_for_background_refresh_settled(&cache_file, &refreshed_body, Duration::from_secs(4)),
        "background refresh did not update the cache"
    );
}

#[test]
fn prompt_segment_missing_cache_returns_immediately_then_renders_refreshed_cache() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = tmp.path().join("usage.json");
    let refreshed_body = usage_json(10.0, 20.0);
    let response_body = refreshed_body.clone();
    let server = TestServer::new(move |_| {
        thread::sleep(Duration::from_millis(800));
        HttpResponse::new(200, response_body.clone())
            .with_header("Content-Type", "application/json")
    })
    .expect("server");
    let options = base_options(tmp.path())
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "missing-cache-token")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        )
        .with_env("NO_COLOR", "1")
        .with_env("TZ", "UTC");

    let started = Instant::now();
    let output = run(
        &["prompt-segment", "--ttl", "1s", "--time-format", "%Y-%m-%d"],
        &options,
    );
    let elapsed = started.elapsed();

    assert_exit(&output, 0);
    assert!(
        elapsed < Duration::from_millis(400),
        "missing-cache prompt path blocked for {elapsed:?}"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");

    assert!(
        wait_for_background_refresh_settled(&cache_file, &refreshed_body, Duration::from_secs(4)),
        "missing-cache background refresh did not create the cache"
    );
    let rendered = run(
        &["prompt-segment", "--ttl", "1h", "--time-format", "%Y-%m-%d"],
        &options,
    );
    assert_exit(&rendered, 0);
    assert_eq!(stdout(&rendered), "5h:90% W:80% 2026-01-03\n");
}
