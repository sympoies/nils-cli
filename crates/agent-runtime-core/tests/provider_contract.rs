use agent_runtime_core::provider::{ProviderAdapter, ProviderAdapterV1};
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateResponse, AuthStateStatus, CapabilitiesRequest,
    CapabilitiesResponse, Capability, ContractVersion, ExecuteRequest, ExecuteResponse,
    HealthStatus, HealthcheckRequest, HealthcheckResponse, LimitsRequest, LimitsResponse,
    ProviderEnvelope, ProviderError, ProviderErrorCategory, ProviderMaturity, ProviderMetadata,
    ProviderOperation, ProviderRef, ProviderResult,
};
use pretty_assertions::assert_eq;

struct MockProvider;

impl ProviderAdapterV1 for MockProvider {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata::new("mock")
    }

    fn capabilities(&self, _request: CapabilitiesRequest) -> ProviderResult<CapabilitiesResponse> {
        Ok(CapabilitiesResponse {
            capabilities: vec![Capability::available("structured-output")],
        })
    }

    fn healthcheck(&self, _request: HealthcheckRequest) -> ProviderResult<HealthcheckResponse> {
        Ok(HealthcheckResponse {
            status: HealthStatus::Healthy,
            summary: Some("ready".to_string()),
            details: None,
        })
    }

    fn execute(&self, request: ExecuteRequest) -> ProviderResult<ExecuteResponse> {
        Ok(ExecuteResponse {
            exit_code: 0,
            stdout: format!("executed: {}", request.task),
            stderr: String::new(),
            duration_ms: Some(12),
        })
    }

    fn limits(&self, _request: LimitsRequest) -> ProviderResult<LimitsResponse> {
        Ok(LimitsResponse {
            max_concurrency: Some(2),
            max_timeout_ms: Some(60_000),
            max_input_bytes: None,
        })
    }

    fn auth_state(&self, _request: AuthStateRequest) -> ProviderResult<AuthStateResponse> {
        Ok(AuthStateResponse {
            state: AuthStateStatus::Authenticated,
            subject: Some("service-account".to_string()),
            scopes: vec!["execute".to_string()],
            expires_at: None,
        })
    }
}

#[test]
fn capabilities_envelope_roundtrip_is_stable() {
    let provider = MockProvider;
    let envelope = provider.capabilities_envelope(CapabilitiesRequest::default());

    let json = serde_json::to_value(&envelope).expect("serialize envelope");
    assert_eq!(json["contract_version"], "provider-adapter.v1");
    assert_eq!(json["provider"]["id"], "mock");
    assert_eq!(json["operation"], "capabilities");
    assert_eq!(json["status"], "ok");
    assert_eq!(
        json["result"]["capabilities"][0]["name"],
        "structured-output"
    );

    let decoded: ProviderEnvelope<CapabilitiesResponse> =
        serde_json::from_value(json).expect("deserialize envelope");
    assert_eq!(decoded, envelope);
}

#[test]
fn provider_metadata_defaults_to_stable_maturity() {
    let metadata = ProviderMetadata::new("mock");
    assert_eq!(metadata.maturity, ProviderMaturity::Stable);
    assert_eq!(metadata.maturity.as_str(), "stable");
}

#[test]
fn auth_state_operation_uses_kebab_case_wire_name() {
    let provider = MockProvider;
    let envelope = provider.auth_state_envelope(AuthStateRequest::default());
    let json = serde_json::to_value(&envelope).expect("serialize envelope");

    assert_eq!(json["operation"], "auth-state");
    assert_eq!(json["result"]["state"], "authenticated");
}

#[test]
fn envelope_compat_defaults_missing_contract_version_to_v1() {
    let legacy = serde_json::json!({
        "provider": { "id": "legacy-mock" },
        "operation": "limits",
        "status": "ok",
        "result": {
            "max_concurrency": 4
        }
    });

    let decoded: ProviderEnvelope<LimitsResponse> =
        serde_json::from_value(legacy).expect("deserialize legacy envelope");
    assert_eq!(decoded.contract_version, ContractVersion::V1);
    assert_eq!(decoded.provider, ProviderRef::new("legacy-mock"));
    assert_eq!(decoded.operation, ProviderOperation::Limits);
}

#[test]
fn error_envelope_and_category_retry_policy_are_stable() {
    let auth_error = ProviderError::new(
        ProviderErrorCategory::Auth,
        "missing-credentials",
        "provider credentials are missing",
    );
    assert!(!auth_error.is_retryable());

    let envelope = ProviderEnvelope::<ExecuteResponse>::from_result(
        ProviderRef::new("mock"),
        ProviderOperation::Execute,
        Err(auth_error.clone().into()),
    );
    let json = serde_json::to_value(&envelope).expect("serialize error envelope");
    assert_eq!(json["status"], "error");
    assert_eq!(json["error"]["category"], "auth");
    assert_eq!(json["error"]["code"], "missing-credentials");

    assert!(ProviderErrorCategory::RateLimit.is_retryable());
    assert!(ProviderErrorCategory::Timeout.is_retryable());
    assert!(ProviderErrorCategory::Network.is_retryable());
    assert!(ProviderErrorCategory::Unavailable.is_retryable());
    assert!(!ProviderErrorCategory::Validation.is_retryable());
    assert!(!ProviderErrorCategory::Dependency.is_retryable());
    assert!(!ProviderErrorCategory::Internal.is_retryable());
    assert!(!ProviderErrorCategory::Unknown.is_retryable());
}

fn assert_provider_alias<T: ProviderAdapter>(_provider: &T) {}

#[test]
fn provider_adapter_alias_accepts_v1_implementor() {
    let provider = MockProvider;
    assert_provider_alias(&provider);
}

#[test]
fn envelope_helpers_cover_all_operations() {
    let provider = MockProvider;

    let health = provider.healthcheck_envelope(HealthcheckRequest::default());
    assert_eq!(health.provider.id, "mock");
    assert_eq!(health.operation, ProviderOperation::Healthcheck);
    assert_eq!(health.contract_version, ContractVersion::V1);
    let health_json = serde_json::to_value(&health).expect("serialize health envelope");
    assert_eq!(health_json["status"], "ok");
    assert_eq!(health_json["result"]["status"], "healthy");

    let execute = provider.execute_envelope(ExecuteRequest::new("run task"));
    assert_eq!(execute.provider.id, "mock");
    assert_eq!(execute.operation, ProviderOperation::Execute);
    let execute_json = serde_json::to_value(&execute).expect("serialize execute envelope");
    assert_eq!(execute_json["status"], "ok");
    assert_eq!(execute_json["result"]["stdout"], "executed: run task");

    let limits = provider.limits_envelope(LimitsRequest::default());
    assert_eq!(limits.provider.id, "mock");
    assert_eq!(limits.operation, ProviderOperation::Limits);
    let limits_json = serde_json::to_value(&limits).expect("serialize limits envelope");
    assert_eq!(limits_json["status"], "ok");
    assert_eq!(limits_json["result"]["max_concurrency"], 2);
}
