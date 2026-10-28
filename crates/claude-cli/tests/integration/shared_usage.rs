use crate::support::*;
#[path = "shared_usage/cached.rs"]
mod cached;
use nils_test_support::http::{HttpResponse, TestServer};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const BODY: &str = r#"{"five_hour":{"utilization":25,"resets_at":"2030-01-01T00:00:00Z"},"seven_day":{"utilization":40,"resets_at":"2030-01-02T00:00:00Z"}}"#;

fn profiles(root: &Path) -> std::path::PathBuf {
    let dir = root.join("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, token, account) in [
        ("alpha", "fixture-alpha", "account-a"),
        ("alias", "fixture-alias", "account-a"),
        ("beta", "fixture-beta", "account-b"),
    ] {
        std::fs::write(
            dir.join(format!("{name}.json")),
            json!({
                "claudeAiOauth": {"accessToken": token, "expiresAt": 4102444800000i64},
                "oauthAccount": {"accountUuid": account}
            })
            .to_string(),
        )
        .unwrap();
    }
    dir
}

#[test]
fn shared_usage_concurrent_mixed_readers_coalesce_success_per_account() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = profiles(tmp.path());
    let server = TestServer::new(|_| HttpResponse::new(200, BODY)).unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_SECRET_DIR", &path_str(&dir))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-alpha")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    for _ in 0..2 {
        std::thread::scope(|scope| {
            for i in 0..12 {
                let options = &options;
                scope.spawn(move || {
                    let args: &[&str] = match i % 3 {
                        0 => &["usage", "--source", "oauth", "--format", "json"],
                        1 => &["diag", "rate-limits", "--all", "--json"],
                        _ => &["prompt-segment", "--refresh"],
                    };
                    assert_exit(&run(args, options), 0);
                });
            }
        });
        assert_eq!(
            server.take_requests().len(),
            2,
            "one fetch for each of two accounts, including different tokens for one account"
        );
        age_shared_usage(tmp.path());
    }
}

#[test]
fn shared_usage_retry_hint_throttles_every_reader_and_account_alias() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = profiles(tmp.path());
    let server = TestServer::new(|_| {
        HttpResponse::new(429, "rate limited").with_header("Retry-After", "7200")
    })
    .unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_SECRET_DIR", &path_str(&dir))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-alpha")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_exit(&run(&["usage", "--source", "oauth"], &options), 0);
    let output = run(&["diag", "rate-limits", "--all"], &options);
    assert_exit(&output, 1);
    assert!(
        stdout(&output).contains("rate limited until"),
        "{}",
        stdout(&output)
    );
    assert_exit(&run(&["prompt-segment", "--refresh"], &options), 0);
    let states: Vec<Value> = std::fs::read_dir(tmp.path().join("usage-backoff"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        .collect();
    assert_eq!(states.len(), 2);
    assert!(
        states
            .iter()
            .all(|state| state["retry_at"].as_u64().unwrap() >= before + 7200)
    );
    assert_eq!(server.take_requests().len(), 2);
}

#[test]
fn shared_usage_throttled_table_reports_cache_age_in_all_modes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = profiles(tmp.path());
    let server = TestServer::new(|_| HttpResponse::new(429, "rate limited")).unwrap();
    // An existing diagnostic cache with a known age, owned by this token.
    use sha2::{Digest, Sha256};
    let key: String = Sha256::digest(b"fixture-alpha")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let cache_dir = tmp.path().join("diag-rate-limits");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(cache_dir.join("alpha.kv"), format!("fetched_at={}\nnon_weekly_label=5h\nnon_weekly_remaining=75\nnon_weekly_reset_epoch=1893456000\nweekly_remaining=60\nweekly_reset_epoch=1893542400\nowner_token_key={key}\n", now-120)).unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_SECRET_DIR", &path_str(&dir))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    for mode in ["--all", "--async"] {
        let output = run(&["diag", "rate-limits", mode], &options);
        let text = stdout(&output);
        let row = text
            .lines()
            .find(|line| line.starts_with("alpha "))
            .unwrap();
        assert!(row.contains("75%"), "{row}");
        assert!(row.contains("rate limited until"), "{row}");
        assert!(row.contains("cache age="), "{row}");
    }
    let path = cache_dir.join("alpha.kv");
    let expired = std::fs::read_to_string(&path).unwrap().replace(
        &format!("fetched_at={}", now - 120),
        &format!("fetched_at={}", now - 1200),
    );
    std::fs::write(path, expired).unwrap();
    for mode in ["--all", "--async"] {
        let output = run(&["diag", "rate-limits", mode], &options);
        let text = stdout(&output);
        let row = text
            .lines()
            .find(|line| line.starts_with("alpha "))
            .unwrap();
        assert!(
            !row.contains("75%"),
            "expired windows must be omitted: {row}"
        );
        assert!(row.contains("rate limited until"), "{row}");
        assert!(
            row.contains("cache age=12"),
            "expired snapshot age remains diagnostic: {row}"
        );
    }
}

#[test]
fn shared_usage_throttled_readers_reuse_one_snapshot_with_original_age() {
    for producer in ["usage", "diag"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = profiles(tmp.path());
        let limited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = limited.clone();
        let server = TestServer::new(move |_| {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                HttpResponse::new(429, "rate limited")
            } else {
                HttpResponse::new(200, BODY)
            }
        })
        .unwrap();
        let options = base_options(tmp.path())
            .with_env("CLAUDE_SECRET_DIR", &path_str(&dir))
            .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
            .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
            .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-alpha")
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            );
        let seed: &[&str] = if producer == "usage" {
            &["usage", "--source", "oauth"]
        } else {
            &["diag", "rate-limits", "alpha", "--json"]
        };
        assert_exit(&run(seed, &options), 0);
        age_shared_usage(tmp.path());
        limited.store(true, std::sync::atomic::Ordering::SeqCst);
        let output = run(
            &["usage", "--source", "oauth", "--format", "json"],
            &options,
        );
        let result = output.stdout_json();
        assert_eq!(result["result"]["reason_code"], "rate_limited");
        assert_eq!(result["result"]["windows"][0]["remaining_percent"], 75.0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(now - result["result"]["updated_at"].as_u64().unwrap() >= 120);
        let output = run(&["diag", "rate-limits", "alias", "--json"], &options);
        let result = output.stdout_json();
        assert_eq!(result["result"]["source"], "cache-fallback");
        assert_eq!(result["result"]["reason_code"], "rate_limited");
        assert_eq!(result["result"]["windows"][0]["remaining_percent"], 75);
        assert_exit(&run(&["prompt-segment", "--refresh"], &options), 0);
        assert_eq!(server.take_requests().len(), 2);
    }
}

#[test]
fn repair_f1_alias_first_read_migrates_another_tokens_account_cooldown() {
    use sha2::{Digest, Sha256};
    let tmp = tempfile::tempdir().unwrap();
    let dir = profiles(tmp.path());
    let state_dir = tmp.path().join("usage-backoff");
    std::fs::create_dir_all(&state_dir).unwrap();
    let token_key: String = Sha256::digest(b"fixture-alpha")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let retry_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 7200;
    std::fs::write(
        state_dir.join(format!("{token_key}.json")),
        json!({"retry_at": retry_at, "delay_seconds": 7200}).to_string(),
    )
    .unwrap();
    let server = TestServer::new(|_| HttpResponse::new(200, BODY)).unwrap();
    let options = base_options(tmp.path())
        .with_env("CLAUDE_SECRET_DIR", &path_str(&dir))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-alias")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    let alias = run(&["diag", "rate-limits", "alias", "--json"], &options);
    assert_eq!(alias.stdout_json()["result"]["reason_code"], "rate_limited");
    assert!(
        server.take_requests().is_empty(),
        "alias must inherit alpha's existing server cooldown before its first HTTP read"
    );
    assert_exit(&run(&["usage", "--source", "oauth"], &options), 0);
    assert_exit(&run(&["prompt-segment", "--refresh"], &options), 0);
    assert!(server.take_requests().is_empty());
    let states: Vec<Value> = std::fs::read_dir(&state_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        .collect();
    assert!(
        states.iter().any(|state| state["retry_at"] == retry_at),
        "migration must not shorten the server deadline"
    );
    assert_exit(
        &run(&["diag", "rate-limits", "beta", "--json"], &options),
        0,
    );
    let unknown = options.with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-unknown");
    assert_exit(&run(&["usage", "--source", "oauth"], &unknown), 0);
    assert_eq!(
        server.take_requests().len(),
        2,
        "another account and an unknown token remain independent"
    );
}
