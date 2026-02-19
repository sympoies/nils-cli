use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateStatus, CapabilitiesRequest, ExecuteRequest, HealthStatus,
    HealthcheckRequest, LimitsRequest, ProviderErrorCategory,
};
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

#[test]
fn missing_auth_reports_stable_non_execute_readiness_reasons() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let capabilities = adapter
        .capabilities(CapabilitiesRequest::default())
        .expect("capabilities");
    let execute = capabilities
        .capabilities
        .iter()
        .find(|capability| capability.name == "execute")
        .expect("execute capability");
    assert_eq!(execute.available, false);
    assert_eq!(
        execute.description.as_deref(),
        Some("set ANTHROPIC_API_KEY to enable execute capability")
    );

    let health = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");
    assert_eq!(health.status, HealthStatus::Degraded);
    let details = health.details.expect("health details");
    assert_eq!(details["readiness_reason_code"], "missing-api-key");
    assert_eq!(details["config_error_code"], "missing-api-key");

    let auth = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");
    assert_eq!(auth.state, AuthStateStatus::Unauthenticated);
}

#[test]
fn invalid_config_injection_reports_stable_non_execute_readiness_reasons() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", "not-a-url");
    let adapter = ClaudeProviderAdapter::new();

    let capabilities = adapter
        .capabilities(CapabilitiesRequest::default())
        .expect("capabilities");
    let execute = capabilities
        .capabilities
        .iter()
        .find(|capability| capability.name == "execute")
        .expect("execute capability");
    assert_eq!(execute.available, false);
    assert!(
        execute
            .description
            .as_deref()
            .unwrap_or_default()
            .contains("invalid-config")
    );

    let health = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");
    assert_eq!(health.status, HealthStatus::Unhealthy);
    let details = health.details.expect("health details");
    assert_eq!(details["readiness_reason_code"], "invalid-config");
    assert_eq!(details["config_error_code"], "invalid-config");
    assert_eq!(details["api_key_configured"], true);

    let limits = adapter.limits(LimitsRequest::default()).expect("limits");
    assert_eq!(limits.max_timeout_ms, None);

    let auth = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");
    assert_eq!(auth.state, AuthStateStatus::Authenticated);
}
