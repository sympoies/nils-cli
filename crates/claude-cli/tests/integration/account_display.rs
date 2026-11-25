use crate::support::{assert_exit, base_options, path_str, run, stdout};
use nils_test_support::cmd::CmdOptions;
use nils_test_support::http::{HttpResponse, LoopbackServer};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const TOKEN: &str = "fixture-display";
const MAX_20X: &str = "default_claude_max_20x";

fn fixture(
    subscription: &str,
    tier: &str,
    cedar: Value,
) -> (tempfile::TempDir, LoopbackServer, CmdOptions) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let profiles = root.join("profiles");
    std::fs::create_dir(&profiles).expect("profiles directory");
    let credentials = json!({
        "claudeAiOauth": {
            "accessToken": TOKEN,
            "subscriptionType": subscription,
            "rateLimitTier": tier
        }
    })
    .to_string();
    let config = root.join("claude-config");
    std::fs::create_dir(&config).expect("config directory");
    std::fs::write(config.join(".credentials.json"), &credentials).expect("active login");
    std::fs::write(
        config.join(".claude.json"),
        json!({"oauthAccount": {"accountUuid": "fixture-account", "emailAddress": "example@example.com"}}).to_string(),
    )
    .expect("active account");
    std::fs::write(profiles.join("alpha.json"), &credentials).expect("profile login");

    let body = json!({
        "five_hour": {"utilization": 6},
        "seven_day": {"utilization": 12, "resets_at": "2023-11-21T20:53:20Z"},
        "cedar_ember": cedar
    })
    .to_string();
    let server = LoopbackServer::new().expect("usage server");
    server.add_route(
        "GET",
        "/usage",
        HttpResponse::new(200, body).with_header("Content-Type", "application/json"),
    );

    let options = base_options(root)
        .with_env("CLAUDE_SECRET_DIR", &path_str(&profiles))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "1.2.3")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", TOKEN)
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        )
        .with_env("NO_COLOR", "1")
        .with_env("TZ", "UTC");
    (tmp, server, options)
}

#[test]
fn prompt_segment_matches_codex_account_layout() {
    let (_tmp, _server, options) = fixture("max", MAX_20X, json!({"eligible": true, "grants": []}));
    let weekly = run(
        &[
            "prompt-segment",
            "--refresh",
            "--no-5h",
            "--time-format",
            "%Y-%m-%dT%H:%MZ",
        ],
        &options,
    );
    assert_exit(&weekly, 0);
    assert_eq!(stdout(&weekly), "alpha W:88% 2023-11-21T20:53Z\n");

    let default = run(&["prompt-segment"], &options);
    assert_exit(&default, 0);
    assert_eq!(stdout(&default), "alpha 5h:94% W:88% 11-21 20:53\n");

    let with_timezone = run(&["prompt-segment", "--show-timezone"], &options);
    assert_exit(&with_timezone, 0);
    assert_eq!(
        stdout(&with_timezone),
        "alpha 5h:94% W:88% 11-21 20:53 +00:00\n"
    );

    let weekly_with_timezone = run(&["prompt-segment", "--no-5h", "--show-timezone"], &options);
    assert_exit(&weekly_with_timezone, 0);
    assert_eq!(
        stdout(&weekly_with_timezone),
        "alpha W:88% 11-21 20:53 +00:00\n"
    );
}

#[test]
fn prompt_segment_uses_only_labels_bound_to_the_usage_login() {
    let (tmp, _server, options) = fixture("max", MAX_20X, json!({"eligible": true, "grants": []}));
    let profile = tmp.path().join("profiles/alpha.json");
    let config = tmp.path().join("claude-config");
    let snapshot = |expected: &str| {
        let output = run(&["prompt-segment", "--refresh", "--no-5h"], &options);
        assert_exit(&output, 0);
        assert_eq!(stdout(&output), expected);
    };

    // A profile with a rotated token still matches the active account identity.
    std::fs::write(
        &profile,
        json!({
            "claudeAiOauth": {"accessToken": "fixture-rotated"},
            "oauthAccount": {"accountUuid": "fixture-account"},
        })
        .to_string(),
    )
    .expect("rotated saved profile");
    snapshot("alpha W:88% 11-21 20:53\n");

    std::fs::remove_file(profile).expect("remove saved profile");
    snapshot("example W:88% 11-21 20:53\n");

    std::fs::write(config.join(".claude-cli-account"), "projected\n").expect("projected profile");
    snapshot("projected W:88% 11-21 20:53\n");

    std::fs::write(
        config.join(".credentials.json"),
        json!({"claudeAiOauth": {"accessToken": "fixture-other"}}).to_string(),
    )
    .expect("different active login");
    snapshot("W:88% 11-21 20:53\n");
}

#[test]
fn account_display_labels_only_max_20x_as_ultra() {
    for (subscription, tier, ultra) in [
        ("max", MAX_20X, true),
        ("max", "default_claude_max_5x", false),
        ("team", MAX_20X, false),
    ] {
        let (_tmp, _server, options) =
            fixture(subscription, tier, json!({"eligible": true, "grants": []}));
        for args in [
            &["diag", "rate-limits", "alpha"][..],
            &["diag", "rate-limits", "--all"][..],
            &["diag", "rate-limits", "--async"][..],
        ] {
            let text = run(args, &options);
            assert_exit(&text, 0);
            assert_eq!(
                stdout(&text).contains("Ultra"),
                ultra,
                "{args:?} {subscription} {tier}"
            );
        }
        let json = run(&["diag", "rate-limits", "alpha", "--json"], &options);
        assert_exit(&json, 0);
        let json: Value = serde_json::from_str(&stdout(&json)).expect("diagnostic JSON");
        assert_eq!(json["result"]["plan"].as_str(), ultra.then_some("Ultra"));
    }
}

#[test]
fn account_display_distinguishes_pending_absent_consumed_and_unknown_grants() {
    for (cedar, available, unknown) in [
        (
            json!({
                "eligible": true,
                "at_limit": false,
                "grants": [{"id": "fixture_grant", "resets_left": 1, "usable_now": false}]
            }),
            true,
            false,
        ),
        (json!({"eligible": true, "grants": []}), false, false),
        (
            json!({"eligible": true, "grants": [{"id": "fixture_grant", "resets_left": 0}]}),
            false,
            false,
        ),
        (Value::Null, false, true),
    ] {
        let (_tmp, _server, options) = fixture("max", MAX_20X, cedar);
        for args in [
            &["prompt-segment", "--refresh"][..],
            &["diag", "rate-limits", "alpha"][..],
            &["diag", "rate-limits", "--all"][..],
        ] {
            let text = run(args, &options);
            assert_exit(&text, 0);
            let line = stdout(&text);
            assert_eq!(
                line.contains("1 reset available"),
                available,
                "{args:?} {line}"
            );
            assert_eq!(
                line.contains("reset availability unknown"),
                unknown,
                "{args:?} {line}"
            );
        }
        let json = run(&["diag", "rate-limits", "alpha", "--json"], &options);
        assert_exit(&json, 0);
        let json: Value = serde_json::from_str(&stdout(&json)).expect("diagnostic JSON");
        assert_eq!(
            json["result"]["reset_credits"]["available_count"].as_i64(),
            (!unknown).then_some(i64::from(available))
        );
        assert_eq!(
            json["result"]["limit_resets"]["cedar_ember"].is_null(),
            unknown
        );
    }
}

#[test]
fn account_display_reports_unknown_for_other_plans() {
    for (subscription, tier) in [("team", MAX_20X), ("max", "default_claude_max_5x")] {
        let (_tmp, _server, options) = fixture(subscription, tier, Value::Null);
        for args in [
            &["diag", "rate-limits", "alpha"][..],
            &["diag", "rate-limits", "--all"][..],
            &["diag", "rate-limits", "--async"][..],
        ] {
            let output = run(args, &options);
            assert_exit(&output, 0);
            let text = stdout(&output);
            assert!(
                text.contains("reset availability unknown"),
                "{subscription} {tier} {args:?}: {text}"
            );
            assert!(!text.contains("Ultra"), "{text}");
        }
    }
}
