use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::ExecuteRequest;

fn live_profile_enabled() -> bool {
    std::env::var("CLAUDE_LIVE_TEST").ok().as_deref() == Some("1")
}

#[test]
#[ignore]
fn live_smoke_executes_against_real_claude_api() {
    if !live_profile_enabled() {
        eprintln!("skipping live smoke test: set CLAUDE_LIVE_TEST=1 to enable");
        return;
    }

    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    if api_key.trim().is_empty() {
        eprintln!("skipping live smoke test: ANTHROPIC_API_KEY is not set");
        return;
    }

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .execute(ExecuteRequest::new(
            "prompt: reply with exactly one word: OK",
        ))
        .expect("live execute");
    assert_eq!(response.exit_code, 0);
    assert!(
        !response.stdout.trim().is_empty(),
        "live response should contain text"
    );
}
