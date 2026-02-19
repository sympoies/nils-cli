use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateStatus, CapabilitiesRequest, ExecuteRequest, HealthStatus,
    HealthcheckRequest, LimitsRequest, ProviderErrorCategory, ProviderMaturity,
};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;

#[test]
fn metadata_reports_stable_maturity() {
    let adapter = ClaudeProviderAdapter::new();
    let metadata = adapter.metadata();

    assert_eq!(metadata.id, "claude");
    assert_eq!(metadata.contract_version.as_str(), "provider-adapter.v1");
    assert_eq!(metadata.maturity, ProviderMaturity::Stable);
}

#[test]
fn capabilities_require_api_key_for_execute() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .capabilities(CapabilitiesRequest::default())
        .expect("capabilities");

    let execute = response
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
            .contains("ANTHROPIC_API_KEY")
    );
}

#[test]
fn capabilities_include_experimental_local_cli_flag() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("claude", "#!/bin/sh\necho claude 0.0.0\n");
    let _path = prepend_path(&lock, stub.path());
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .capabilities(CapabilitiesRequest {
            include_experimental: true,
        })
        .expect("capabilities");

    let execute = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "execute")
        .expect("execute capability");
    assert_eq!(execute.available, true);

    let local_cli = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "characterization.local-cli")
        .expect("local cli capability");
    assert_eq!(local_cli.available, true);
}

#[test]
fn healthcheck_is_degraded_when_api_key_is_missing() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Degraded);
    assert_eq!(
        response.summary.as_deref(),
        Some("claude adapter is partially ready")
    );
    let details = response.details.expect("details");
    assert_eq!(details["api_key_configured"], false);
    assert_eq!(details["execute_available"], false);
}

#[test]
fn healthcheck_is_healthy_when_api_key_is_present() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest {
            timeout_ms: Some(1500),
        })
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Healthy);
    assert_eq!(response.summary.as_deref(), Some("claude adapter is ready"));
    let details = response.details.expect("details");
    assert_eq!(details["api_key_configured"], true);
    assert_eq!(details["requested_timeout_ms"], 1500);
}

#[test]
fn execute_returns_validation_error_for_blank_prompt() {
    let adapter = ClaudeProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest {
            task: "   ".to_string(),
            input: Some("".to_string()),
            timeout_ms: None,
        })
        .expect_err("expected missing-task");

    assert_eq!(error.category, ProviderErrorCategory::Validation);
    assert_eq!(error.code, "missing-task");
}

#[test]
fn execute_returns_auth_error_when_api_key_is_missing() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest::new("ping"))
        .expect_err("expected auth error");

    assert_eq!(error.category, ProviderErrorCategory::Auth);
    assert_eq!(error.code, "missing-api-key");
}

#[test]
fn limits_report_configurable_concurrency() {
    let lock = GlobalStateLock::new();
    let _concurrency = EnvGuard::set(&lock, "CLAUDE_MAX_CONCURRENCY", "4");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter.limits(LimitsRequest::default()).expect("limits");

    assert_eq!(response.max_concurrency, Some(4));
}

#[test]
fn auth_state_tracks_api_key_presence() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let unauth = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");
    assert_eq!(unauth.state, AuthStateStatus::Unauthenticated);

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key-1234");
    let _subject = EnvGuard::set(&lock, "ANTHROPIC_AUTH_SUBJECT", "claude-user@example.com");
    let _scopes = EnvGuard::set(
        &lock,
        "ANTHROPIC_AUTH_SCOPES",
        "messages:read,messages:write",
    );
    let auth = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");
    assert_eq!(auth.state, AuthStateStatus::Authenticated);
    assert_eq!(auth.subject.as_deref(), Some("claude-user@example.com"));
    assert_eq!(
        auth.scopes,
        vec!["messages:read".to_string(), "messages:write".to_string()]
    );
}
