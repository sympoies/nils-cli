use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateStatus, CapabilitiesRequest, ExecuteRequest, HealthStatus,
    HealthcheckRequest, LimitsRequest, ProviderErrorCategory, ProviderMaturity,
};
use pretty_assertions::assert_eq;

#[test]
fn metadata_reports_stub_maturity() {
    let adapter = ClaudeProviderAdapter::new();
    let metadata = adapter.metadata();

    assert_eq!(metadata.id, "claude");
    assert_eq!(metadata.contract_version.as_str(), "provider-adapter.v1");
    assert_eq!(metadata.maturity, ProviderMaturity::Stub);
}

#[test]
fn capabilities_report_stub_execute_and_auth_state() {
    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .capabilities(CapabilitiesRequest::default())
        .expect("capabilities");

    assert_eq!(response.capabilities.len(), 5);
    let execute = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "execute")
        .expect("execute capability");
    assert_eq!(execute.available, false);
    assert_eq!(
        execute.description.as_deref(),
        Some("stub provider adapter; execute is not implemented")
    );

    let auth_state = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "auth-state")
        .expect("auth-state capability");
    assert_eq!(auth_state.available, false);
}

#[test]
fn healthcheck_reports_stub_degraded_state() {
    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Degraded);
    assert_eq!(
        response.summary.as_deref(),
        Some("claude provider adapter is a stub")
    );
    let details = response.details.expect("details");
    assert_eq!(details["maturity"], "stub");
    assert_eq!(details["execute_available"], false);
}

#[test]
fn execute_returns_not_implemented_unavailable_error() {
    let adapter = ClaudeProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest::new("ping"))
        .expect_err("expected stub execute error");

    assert_eq!(error.category, ProviderErrorCategory::Unavailable);
    assert_eq!(error.code, "not-implemented");
}

#[test]
fn limits_report_single_concurrency() {
    let adapter = ClaudeProviderAdapter::new();
    let response = adapter.limits(LimitsRequest::default()).expect("limits");

    assert_eq!(response.max_concurrency, Some(1));
    assert_eq!(response.max_timeout_ms, None);
    assert_eq!(response.max_input_bytes, None);
}

#[test]
fn auth_state_is_unknown_for_stub_provider() {
    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");

    assert_eq!(response.state, AuthStateStatus::Unknown);
    assert_eq!(response.subject, None);
}
