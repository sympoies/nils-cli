use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{ExecuteRequest, ProviderErrorCategory, ProviderMaturity};
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
fn execute_returns_not_implemented_unavailable_error() {
    let adapter = ClaudeProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest::new("ping"))
        .expect_err("expected stub execute error");

    assert_eq!(error.category, ProviderErrorCategory::Unavailable);
    assert_eq!(error.code, "not-implemented");
}
