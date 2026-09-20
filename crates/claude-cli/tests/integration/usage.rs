use crate::support::*;
use nils_test_support::http::{HttpResponse, LoopbackServer, TestServer};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
#[test]
fn usage_auto_falls_back_to_claude_cli_and_writes_cache() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/usr/bin/env sh
cat >/dev/null
cat <<'OUT'
Claude Code Usage

Current session
5-hour limit: 25% used, 75% remaining
Resets at 2026-01-01T00:00:00+00:00

Current week
Weekly limit: 50% used, 50% remaining
Resets at 2026-01-03T12:30:00+00:00
OUT
"#,
    );
    let server = TestServer::new(|_| {
        thread::sleep(Duration::from_millis(400));
        HttpResponse::new(500, "upstream exploded")
    })
    .expect("server");

    let output = run(
        &["usage", "--format", "json", "--source", "auto"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "secret-token-usage")
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            )
            .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_DISABLED", "1"),
    );

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("secret-token-usage"));
    assert!(!stderr(&output).contains("secret-token-usage"));

    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "claude-cli.usage.v1");
    assert_eq!(payload["command"], "usage");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["provider"], "claude");
    assert_eq!(payload["result"]["source"], "cli");
    assert_eq!(payload["result"]["stale"], false);
    assert_eq!(payload["result"]["windows"][0]["key"], "5h");
    assert_eq!(payload["result"]["windows"][0]["used_percent"], 25.0);
    assert_eq!(payload["result"]["windows"][0]["remaining_percent"], 75.0);
    assert_eq!(payload["result"]["windows"][1]["key"], "weekly");
    assert_eq!(payload["result"]["windows"][1]["used_percent"], 50.0);
    assert_eq!(payload["result"]["windows"][1]["remaining_percent"], 50.0);

    let cached = std::fs::read_to_string(tmp.path().join("usage.json")).expect("cache");
    assert!(!cached.contains("secret-token-usage"));
    assert!(cached.contains("\"five_hour\""));
    assert!(cached.contains("\"seven_day\""));
}

#[test]
fn usage_oauth_classifies_past_due_billing_without_forwarding_provider_body() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = LoopbackServer::new().expect("server");
    let provider_body = r#"{"error":{"message":"Your subscription payment is past due. Please pay your overdue invoice to restore access."}}"#;
    server.add_route("GET", "/usage", HttpResponse::new(402, provider_body));

    let output = run(
        &["usage", "--format", "json", "--source", "oauth"],
        &base_options(tmp.path())
            .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "secret-token-billing")
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            ),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["reason_code"], "billing_past_due");
    assert!(!stdout(&output).contains("overdue invoice"));
    assert!(!stdout(&output).contains("secret-token-billing"));
}

#[test]
fn usage_oauth_classifies_missing_auth() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = run(
        &["usage", "--format", "json", "--source", "oauth"],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["reason_code"], "auth_required");
}

#[cfg(unix)]
#[test]
fn usage_cli_classifies_organization_disabled_without_forwarding_terminal_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/usr/bin/env sh
cat >/dev/null
printf '%s\n' 'Your organization has disabled Claude subscription access for Claude Code. Contact your admin.'
"#,
    );

    let output = run(
        &["usage", "--format", "json", "--source", "cli"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_DISABLED", "1"),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["reason_code"], "organization_disabled");
    assert!(!stdout(&output).contains("Contact your admin"));
}

#[cfg(unix)]
#[test]
fn usage_auto_prefers_recent_structured_api_error_over_generic_usage_failure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("claude-config");
    let transcript = config_dir.join("projects/repo/session.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("parent")).expect("projects");
    std::fs::write(
        &transcript,
        r#"{"type":"assistant","isApiErrorMessage":true,"message":{"type":"message","content":[{"type":"text","text":"Your organization has disabled Claude subscription access for Claude Code. Contact your admin."}]}}
"#,
    )
    .expect("transcript");
    let bin_dir = write_fake_claude(
        tmp.path(),
        "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' 'usage unavailable'\n",
    );
    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/usage", HttpResponse::new(429, "rate limited"));

    let output = run(
        &["usage", "--format", "json", "--source", "auto"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_CONFIG_DIR", &path_str(&config_dir))
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN",
                "secret-token-api-error",
            )
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            )
            .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_DISABLED", "1"),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["reason_code"], "organization_disabled");
    assert!(!stdout(&output).contains("Contact your admin"));
    assert!(!stdout(&output).contains("session.jsonl"));
}

#[cfg(unix)]
#[test]
fn usage_auto_ignores_structured_api_error_after_newer_assistant_success() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("claude-config");
    let transcript = config_dir.join("projects/repo/session.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("parent")).expect("projects");
    std::fs::write(
        &transcript,
        concat!(
            r#"{"type":"assistant","isApiErrorMessage":true,"message":{"type":"message","content":[{"type":"text","text":"Your organization has disabled Claude subscription access for Claude Code. Contact your admin."}]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"type":"message","content":[{"type":"text","text":"Access restored."}]}}"#,
            "\n"
        ),
    )
    .expect("transcript");
    let bin_dir = write_fake_claude(
        tmp.path(),
        "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' 'usage unavailable'\n",
    );
    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/usage", HttpResponse::new(429, "rate limited"));

    let output = run(
        &["usage", "--format", "json", "--source", "auto"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_CONFIG_DIR", &path_str(&config_dir))
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN",
                "secret-token-api-error",
            )
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            )
            .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_DISABLED", "1"),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["reason_code"], "rate_limited");
    assert!(!stdout(&output).contains("Contact your admin"));
}

#[cfg(unix)]
#[test]
fn usage_auto_ignores_older_error_after_newer_success_in_another_transcript() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("claude-config");
    let projects = config_dir.join("projects/repo");
    std::fs::create_dir_all(&projects).expect("projects");
    let error_transcript = projects.join("older-error.jsonl");
    std::fs::write(
        &error_transcript,
        concat!(
            r#"{"type":"assistant","isApiErrorMessage":true,"message":{"content":[{"text":"Your organization has disabled Claude access."}]}}"#,
            "\n"
        ),
    )
    .expect("error transcript");
    let success_transcript = projects.join("newer-success.jsonl");
    std::fs::write(
        &success_transcript,
        concat!(
            r#"{"type":"assistant","message":{"content":[{"text":"Access restored."}]}}"#,
            "\n"
        ),
    )
    .expect("success transcript");
    let now = SystemTime::now();
    set_modified(&error_transcript, now - Duration::from_secs(60));
    set_modified(&success_transcript, now);
    let bin_dir = write_fake_claude(
        tmp.path(),
        "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' 'usage unavailable'\n",
    );
    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/usage", HttpResponse::new(429, "rate limited"));

    let output = run(
        &["usage", "--format", "json", "--source", "auto"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_CONFIG_DIR", &path_str(&config_dir))
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN",
                "secret-token-api-error",
            )
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            )
            .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_DISABLED", "1"),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["reason_code"], "rate_limited");
    assert!(!stdout(&output).contains("organization_disabled"));
}

#[test]
fn usage_cache_source_outputs_epoch_for_rfc3339_reset_times() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(
        tmp.path(),
        &usage_json_with_resets(21.0, 10.0, "2026-07-14T07:20:00Z", "2026-07-18T13:00:00Z"),
    );
    let updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_secs();
    set_modified(&cache_file, UNIX_EPOCH + Duration::from_secs(updated_at));

    let output = run(
        &["usage", "--format", "json", "--source", "cache"],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "claude-cli.usage.v1");
    assert_eq!(payload["command"], "usage");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["updated_at"], updated_at);
    assert_eq!(
        payload["result"]["windows"][0]["resets_at"],
        "2026-07-14T07:20:00Z"
    );
    assert_eq!(
        payload["result"]["windows"][0]["resets_at_epoch"],
        1_784_013_600
    );
    assert_eq!(
        payload["result"]["windows"][1]["resets_at"],
        "2026-07-18T13:00:00Z"
    );
    assert_eq!(
        payload["result"]["windows"][1]["resets_at_epoch"],
        1_784_379_600
    );
}

#[test]
fn usage_cache_source_omits_windows_older_than_max_stale_age() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    set_modified(&cache_file, SystemTime::now() - Duration::from_secs(601));

    let output = run(
        &["usage", "--format", "json", "--source", "cache"],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["windows"], serde_json::json!([]));
    assert!(
        cache_file.is_file(),
        "max-stale handling must not delete cache"
    );
}

#[cfg(unix)]
#[test]
fn usage_auto_keeps_live_rate_limit_reason_when_expired_cache_is_omitted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(25.0, 50.0));
    set_modified(&cache_file, SystemTime::now() - Duration::from_secs(601));
    let bin_dir = write_fake_claude(
        tmp.path(),
        "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' 'usage unavailable'\n",
    );
    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/usage", HttpResponse::new(429, "rate limited"));

    let output = run(
        &["usage", "--format", "json", "--source", "auto"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "secret-token")
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            )
            .with_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_PTY_DISABLED", "1"),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["result"]["windows"], serde_json::json!([]));
    assert_eq!(payload["result"]["reason_code"], "rate_limited");
    assert!(
        cache_file.is_file(),
        "max-stale handling must not delete cache"
    );
}

#[test]
fn usage_clear_cache_removes_only_the_cache_file_before_querying() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(21.0, 10.0));
    let lock_file = refresh_lock_path(&cache_file);
    std::fs::write(&lock_file, "").expect("write refresh lock");

    let output = run(
        &["usage", "--clear-cache", "--source", "oauth"],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 0);
    assert!(!cache_file.exists(), "--clear-cache must remove the cache");
    assert!(
        lock_file.is_file(),
        "--clear-cache must not remove the refresh lock"
    );
    assert!(
        tmp.path().is_dir(),
        "--clear-cache must not remove the cache directory"
    );
}

#[test]
fn usage_clear_cache_with_cache_source_is_a_usage_error_that_keeps_the_cache() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), &usage_json(21.0, 10.0));

    let output = run(
        &["usage", "--clear-cache", "--source", "cache"],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 64);
    assert!(
        cache_file.is_file(),
        "a rejected flag combination must not clear the cache"
    );
    assert!(stdout(&output).is_empty());
    assert!(
        stderr(&output).contains("--clear-cache is not compatible with --source cache"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn usage_clear_cache_json_rejection_is_one_versioned_error_envelope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_cache(tmp.path(), &usage_json(21.0, 10.0));

    let output = run(
        &[
            "usage",
            "--clear-cache",
            "--source",
            "cache",
            "--format",
            "json",
        ],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 64);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "claude-cli.usage.v1");
    assert_eq!(payload["command"], "usage");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "invalid-flag-combination");
}

#[test]
fn usage_debug_reports_bounded_source_attempts_on_stderr_only() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let output = run(
        &["usage", "--debug", "--source", "oauth", "--format", "json"],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 0);

    // stdout stays exactly one versioned envelope, so `claude-cli.usage.v1`
    // consumers are unaffected by debug mode.
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "claude-cli.usage.v1");
    assert_eq!(payload["result"]["reason_code"], "auth_required");

    let stderr_text = stderr(&output);
    let debug_lines: Vec<&str> = stderr_text
        .lines()
        .filter(|line| line.contains("claude-cli usage: debug:"))
        .collect();
    assert_eq!(debug_lines.len(), 1, "stderr: {stderr_text}");
    let line = debug_lines[0];
    assert!(
        line.starts_with(
            "claude-cli usage: debug: source=oauth outcome=unavailable reason=auth_required elapsed_ms="
        ),
        "unexpected debug line: {line}"
    );
    assert!(
        line.rsplit("elapsed_ms=")
            .next()
            .is_some_and(|elapsed| elapsed.parse::<u128>().is_ok()),
        "elapsed_ms must be a number: {line}"
    );
}

#[test]
fn usage_debug_traces_every_attempted_source_without_leaking_private_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let output = run(&["usage", "--debug"], &base_options(tmp.path()));

    assert_exit(&output, 0);
    let stderr_text = stderr(&output);
    for source in ["oauth", "cli", "transcript", "cache"] {
        assert!(
            stderr_text.contains(&format!("source={source} outcome=")),
            "missing debug trace for {source}: {stderr_text}"
        );
    }
    assert!(
        !stderr_text.contains(&path_str(tmp.path())),
        "debug output must not print fixture paths: {stderr_text}"
    );
}
