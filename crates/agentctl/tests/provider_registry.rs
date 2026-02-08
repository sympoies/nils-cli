use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateResponse, CapabilitiesRequest, CapabilitiesResponse, ExecuteRequest,
    ExecuteResponse, HealthStatus, HealthcheckRequest, HealthcheckResponse, LimitsRequest,
    LimitsResponse, ProviderMetadata, ProviderResult,
};
use agentctl::provider::registry::{
    ProviderRegistry, ProviderSelectionSource, ResolveProviderError,
};
use pretty_assertions::assert_eq;

#[test]
fn registry_iter_is_sorted_and_default_falls_back_deterministically() {
    let mut registry = ProviderRegistry::new("codex");
    registry.register(FakeProvider::new("zeta", HealthStatus::Healthy));
    registry.register(FakeProvider::new("alpha", HealthStatus::Degraded));
    registry.register(FakeProvider::new("beta", HealthStatus::Unhealthy));

    let providers = registry
        .iter()
        .map(|(provider_id, _)| provider_id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(providers, vec!["alpha", "beta", "zeta"]);
    assert_eq!(registry.default_provider_id(), Some("alpha"));
}

#[test]
fn registry_prefers_configured_default_when_present() {
    let mut registry = ProviderRegistry::new("codex");
    registry.register(FakeProvider::new("beta", HealthStatus::Healthy));
    registry.register(FakeProvider::new("codex", HealthStatus::Healthy));
    registry.register(FakeProvider::new("alpha", HealthStatus::Healthy));

    assert_eq!(registry.default_provider_id(), Some("codex"));
}

#[test]
fn selection_uses_cli_override_before_environment_override() {
    let mut registry = ProviderRegistry::new("codex");
    registry.register(FakeProvider::new("codex", HealthStatus::Healthy));
    registry.register(FakeProvider::new("alpha", HealthStatus::Healthy));

    let selection = registry
        .resolve_selection_with_env(Some("alpha"), Some("codex"))
        .expect("selection");
    assert_eq!(selection.provider_id, "alpha");
    assert_eq!(selection.source, ProviderSelectionSource::CliArgument);
}

#[test]
fn selection_uses_environment_override_when_cli_override_absent() {
    let mut registry = ProviderRegistry::new("codex");
    registry.register(FakeProvider::new("codex", HealthStatus::Healthy));
    registry.register(FakeProvider::new("alpha", HealthStatus::Healthy));

    let selection = registry
        .resolve_selection_with_env(None, Some("alpha"))
        .expect("selection");
    assert_eq!(selection.provider_id, "alpha");
    assert_eq!(selection.source, ProviderSelectionSource::Environment);
}

#[test]
fn selection_rejects_unknown_override_and_exposes_source() {
    let mut registry = ProviderRegistry::new("codex");
    registry.register(FakeProvider::new("codex", HealthStatus::Healthy));

    let error = registry
        .resolve_selection_with_env(Some("missing"), None)
        .expect_err("should fail");
    assert_eq!(
        error,
        ResolveProviderError::UnknownProvider {
            provider_id: "missing".to_string(),
            source: ProviderSelectionSource::CliArgument,
        }
    );
}

struct FakeProvider {
    id: String,
    status: HealthStatus,
}

impl FakeProvider {
    fn new(id: &str, status: HealthStatus) -> Self {
        Self {
            id: id.to_string(),
            status,
        }
    }
}

impl ProviderAdapterV1 for FakeProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata::new(self.id.clone())
    }

    fn capabilities(&self, _request: CapabilitiesRequest) -> ProviderResult<CapabilitiesResponse> {
        Ok(CapabilitiesResponse::default())
    }

    fn healthcheck(&self, _request: HealthcheckRequest) -> ProviderResult<HealthcheckResponse> {
        Ok(HealthcheckResponse {
            status: self.status,
            summary: None,
            details: None,
        })
    }

    fn execute(&self, _request: ExecuteRequest) -> ProviderResult<ExecuteResponse> {
        Ok(ExecuteResponse::default())
    }

    fn limits(&self, _request: LimitsRequest) -> ProviderResult<LimitsResponse> {
        Ok(LimitsResponse::default())
    }

    fn auth_state(&self, _request: AuthStateRequest) -> ProviderResult<AuthStateResponse> {
        Ok(AuthStateResponse::default())
    }
}
