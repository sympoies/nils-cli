use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateResponse, AuthStateStatus, CapabilitiesRequest,
    CapabilitiesResponse, Capability, ExecuteRequest, ExecuteResponse, HealthStatus,
    HealthcheckRequest, HealthcheckResponse, LimitsRequest, LimitsResponse, ProviderError,
    ProviderErrorCategory, ProviderMaturity, ProviderMetadata, ProviderResult,
};

const PROVIDER_ID: &str = "gemini";

#[derive(Debug, Clone, Default)]
pub struct GeminiProviderAdapter;

impl GeminiProviderAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderAdapterV1 for GeminiProviderAdapter {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata::new(PROVIDER_ID).with_maturity(ProviderMaturity::Stub)
    }

    fn capabilities(&self, _request: CapabilitiesRequest) -> ProviderResult<CapabilitiesResponse> {
        Ok(CapabilitiesResponse {
            capabilities: vec![
                Capability::available("capabilities"),
                Capability::available("healthcheck"),
                Capability {
                    name: "execute".to_string(),
                    available: false,
                    description: Some(
                        "stub provider adapter; execute is not implemented".to_string(),
                    ),
                },
                Capability::available("limits"),
                Capability {
                    name: "auth-state".to_string(),
                    available: false,
                    description: Some(
                        "stub provider adapter; auth-state is not implemented".to_string(),
                    ),
                },
            ],
        })
    }

    fn healthcheck(&self, _request: HealthcheckRequest) -> ProviderResult<HealthcheckResponse> {
        Ok(HealthcheckResponse {
            status: HealthStatus::Degraded,
            summary: Some("gemini provider adapter is a stub".to_string()),
            details: Some(serde_json::json!({
                "maturity": "stub",
                "execute_available": false,
            })),
        })
    }

    fn execute(&self, _request: ExecuteRequest) -> ProviderResult<ExecuteResponse> {
        Err(Box::new(
            ProviderError::new(
                ProviderErrorCategory::Unavailable,
                "not-implemented",
                "gemini provider adapter is a stub and does not implement execute",
            )
            .with_retryable(false),
        ))
    }

    fn limits(&self, _request: LimitsRequest) -> ProviderResult<LimitsResponse> {
        Ok(LimitsResponse {
            max_concurrency: Some(1),
            max_timeout_ms: None,
            max_input_bytes: None,
        })
    }

    fn auth_state(&self, _request: AuthStateRequest) -> ProviderResult<AuthStateResponse> {
        Ok(AuthStateResponse {
            state: AuthStateStatus::Unknown,
            subject: None,
            scopes: Vec::new(),
            expires_at: None,
        })
    }
}
