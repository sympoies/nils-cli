use crate::support::{assert_exit, base_options, path_str, run, stdout};
use nils_test_support::cmd::CmdOptions;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const TOKEN: &str = "fixture-display";
const MAX_20X: &str = "default_claude_max_20x";

fn fixture(subscription: &str, tier: &str, cedar: Value) -> (tempfile::TempDir, CmdOptions) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let profiles = root.join("profiles");
    let shared = root.join("usage-backoff");
    for dir in [&profiles, &shared] {
        std::fs::create_dir(dir).expect("fixture directory");
    }
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
    std::fs::write(profiles.join("sample.json"), &credentials).expect("profile login");

    let key: String = Sha256::digest(TOKEN.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let body = json!({
        "five_hour": {"utilization": 25},
        "seven_day": {"utilization": 50},
        "cedar_ember": cedar
    })
    .to_string();
    std::fs::write(
        shared.join(format!("{key}.response")),
        json!({"fetched_at": chrono::Utc::now().timestamp(), "body": body}).to_string(),
    )
    .expect("saved usage response");

    let options = base_options(root)
        .with_env("CLAUDE_SECRET_DIR", &path_str(&profiles))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "1.2.3")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", TOKEN)
        .with_env("CLAUDE_PROMPT_SEGMENT_ENDPOINT", "file:///unused")
        .with_env("NO_COLOR", "1");
    (tmp, options)
}

#[test]
fn account_display_labels_only_max_20x_as_ultra() {
    for (subscription, tier, ultra) in [
        ("max", MAX_20X, true),
        ("max", "default_claude_max_5x", false),
        ("team", MAX_20X, false),
    ] {
        let (_tmp, options) = fixture(subscription, tier, json!({"eligible": true, "grants": []}));
        let prompt = run(&["prompt-segment", "--refresh"], &options);
        assert_exit(&prompt, 0);
        assert_eq!(
            stdout(&prompt).contains("Ultra"),
            ultra,
            "{subscription} {tier}"
        );
        let cached_prompt = run(&["prompt-segment"], &options);
        assert_exit(&cached_prompt, 0);
        assert_eq!(stdout(&cached_prompt), stdout(&prompt));

        for args in [
            &["diag", "rate-limits", "sample"][..],
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
        let json = run(&["diag", "rate-limits", "sample", "--json"], &options);
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
        let (_tmp, options) = fixture("max", MAX_20X, cedar);
        for args in [
            &["prompt-segment", "--refresh"][..],
            &["diag", "rate-limits", "sample"][..],
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
        let json = run(&["diag", "rate-limits", "sample", "--json"], &options);
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
