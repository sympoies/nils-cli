use super::*;
use pretty_assertions::assert_eq;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[test]
fn shared_usage_offline_cached_text_imports_unseeded_alias() {
    for mode in ["single", "--all", "--async"] {
        offline_cached_alias(mode, false);
    }
}

#[test]
fn shared_usage_offline_cached_json_imports_unseeded_alias() {
    offline_cached_alias("--async", true);
}

fn offline_cached_alias(mode: &str, json_output: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = profiles(tmp.path());
    // Keep the collection focused on two known aliases of the same account.
    std::fs::remove_file(dir.join("beta.json")).unwrap();
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
        .with_env("CLAUDE_SECRET_DIR", &path_str(&dir))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "fixture-alpha")
        .with_env(
            "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
            &format!("{}/usage", server.url()),
        );
    assert_exit(&run(&["usage", "--source", "oauth"], &options), 0);
    assert_eq!(server.take_requests().len(), 1);
    age_shared_usage(tmp.path());
    limited.store(true, Ordering::SeqCst);
    let cooldown = run(
        &["usage", "--source", "oauth", "--format", "json"],
        &options,
    );
    assert_exit(&cooldown, 0);
    let original_time = cooldown.stdout_json()["result"]["updated_at"]
        .as_u64()
        .unwrap();
    assert_eq!(
        cooldown.stdout_json()["result"]["reason_code"],
        "rate_limited"
    );
    assert_eq!(server.take_requests().len(), 1);
    let projection = tmp.path().join("diag-rate-limits/alias.kv");
    assert!(!projection.exists(), "alias must never have been seeded");

    let mut args = vec!["diag", "rate-limits", "--cached"];
    args.push(if mode == "single" { "alias" } else { mode });
    if json_output {
        args.extend(["--format", "json"]);
    }
    let output = run(&args, &options);
    assert!(
        server.take_requests().is_empty(),
        "offline readers must not request"
    );
    assert_exit(&output, 0);
    if json_output {
        let payload = output.stdout_json();
        let alias = payload["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|result| result["name"] == "alias")
            .unwrap();
        assert_eq!(alias["source"], "cache");
        assert_eq!(alias["ok"], true);
        assert_eq!(alias["reason_code"], "rate_limited");
        assert_eq!(alias["windows"][0]["remaining_percent"], 75);
    } else {
        let text = stdout(&output);
        assert!(text.contains("75%"), "{text}");
        if mode != "single" {
            let row = text
                .lines()
                .find(|line| line.starts_with("alias "))
                .unwrap();
            assert!(row.contains("rate limited until"), "{row}");
            let age: u64 = row
                .split("cache age=")
                .nth(1)
                .unwrap()
                .split('s')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            assert!(age >= 120 && age <= now - original_time, "{row}");
        }
    }
    let projected = std::fs::read_to_string(&projection).unwrap();
    assert!(
        projected
            .lines()
            .any(|line| line == format!("fetched_at={original_time}")),
        "offline import must retain the original observation time: {projected}"
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(now - original_time >= 120);
}
