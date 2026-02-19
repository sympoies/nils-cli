use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{ExecuteRequest, ProviderErrorCategory};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::path::PathBuf;

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
}

fn fixture_text(relative: &str) -> String {
    std::fs::read_to_string(fixture_path(relative)).expect("fixture text")
}

#[test]
fn characterization_manifest_contains_required_case_ids() {
    let raw = fixture_text("characterization/manifest.json");
    let parsed: Value = serde_json::from_str(raw.as_str()).expect("manifest json");
    let ids = parsed["cases"]
        .as_array()
        .expect("cases array")
        .iter()
        .filter_map(|case| case.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    for required in [
        "success",
        "auth_failure",
        "rate_limit",
        "timeout",
        "malformed_response",
    ] {
        assert!(ids.contains(&required), "missing fixture id: {required}");
    }
}

#[test]
fn fixture_backed_rate_limit_error_maps_to_retryable_category() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(429, fixture_text("api/rate_limit_error.json")),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let adapter = ClaudeProviderAdapter::new();

    let error = adapter
        .execute(ExecuteRequest::new("prompt: ping"))
        .expect_err("rate limit error expected");
    assert_eq!(error.category, ProviderErrorCategory::RateLimit);
    assert_eq!(error.code, "rate-limited");
    assert_eq!(error.retryable, Some(true));
}

#[test]
fn fixture_backed_auth_error_maps_to_non_retryable_auth_category() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(401, fixture_text("api/auth_error.json")),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let adapter = ClaudeProviderAdapter::new();

    let error = adapter
        .execute(ExecuteRequest::new("prompt: ping"))
        .expect_err("auth error expected");
    assert_eq!(error.category, ProviderErrorCategory::Auth);
    assert_eq!(error.code, "auth-failed");
    assert_eq!(error.retryable, Some(false));
}
