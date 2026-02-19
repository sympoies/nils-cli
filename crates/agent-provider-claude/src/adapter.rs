use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateResponse, AuthStateStatus, CapabilitiesRequest,
    CapabilitiesResponse, Capability, ExecuteRequest, ExecuteResponse, HealthStatus,
    HealthcheckRequest, HealthcheckResponse, LimitsRequest, LimitsResponse, ProviderMaturity,
    ProviderMetadata, ProviderResult,
};
use claude_core::config;
use claude_core::exec;
use serde_json::json;

const PROVIDER_ID: &str = "claude";

#[derive(Debug, Clone, Default)]
pub struct ClaudeProviderAdapter;

impl ClaudeProviderAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderAdapterV1 for ClaudeProviderAdapter {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata::new(PROVIDER_ID).with_maturity(ProviderMaturity::Stable)
    }

    fn capabilities(&self, request: CapabilitiesRequest) -> ProviderResult<CapabilitiesResponse> {
        let execute_enabled = config::ClaudeConfig::from_env().is_ok();
        let execute_description = if execute_enabled {
            "claude execution is enabled".to_string()
        } else {
            format!("set {} to enable execute capability", config::API_KEY_ENV)
        };

        let mut capabilities = vec![
            Capability::available("capabilities"),
            Capability::available("healthcheck"),
            Capability {
                name: "execute".to_string(),
                available: execute_enabled,
                description: Some(execute_description),
            },
            Capability::available("limits"),
            Capability::available("auth-state"),
        ];

        if request.include_experimental {
            capabilities.push(Capability {
                name: "api.messages".to_string(),
                available: execute_enabled,
                description: Some("Direct Anthropic messages API integration".to_string()),
            });
            capabilities.push(Capability {
                name: "characterization.local-cli".to_string(),
                available: config::claude_cli_available(),
                description: Some("Local Claude CLI characterization support".to_string()),
            });
        }

        Ok(CapabilitiesResponse { capabilities })
    }

    fn healthcheck(&self, request: HealthcheckRequest) -> ProviderResult<HealthcheckResponse> {
        let claude_cli_available = config::claude_cli_available();
        match config::ClaudeConfig::from_env() {
            Ok(cfg) => Ok(HealthcheckResponse {
                status: HealthStatus::Healthy,
                summary: Some("claude adapter is ready".to_string()),
                details: Some(json!({
                    "maturity": "stable",
                    "execute_available": true,
                    "api_key_configured": true,
                    "base_url": cfg.base_url,
                    "model": cfg.model,
                    "api_version": cfg.api_version,
                    "timeout_ms": cfg.timeout_ms,
                    "claude_cli_available": claude_cli_available,
                    "requested_timeout_ms": request.timeout_ms,
                })),
            }),
            Err(error) if error.code == "missing-api-key" => Ok(HealthcheckResponse {
                status: HealthStatus::Degraded,
                summary: Some("claude adapter is partially ready".to_string()),
                details: Some(json!({
                    "maturity": "stable",
                    "execute_available": false,
                    "api_key_configured": false,
                    "config_error_code": error.code,
                    "config_error_message": error.message,
                    "claude_cli_available": claude_cli_available,
                    "requested_timeout_ms": request.timeout_ms,
                })),
            }),
            Err(error) => Ok(HealthcheckResponse {
                status: HealthStatus::Unhealthy,
                summary: Some("claude adapter is unavailable".to_string()),
                details: Some(json!({
                    "maturity": "stable",
                    "execute_available": false,
                    "api_key_configured": config::api_key_configured(),
                    "config_error_code": error.code,
                    "config_error_message": error.message,
                    "claude_cli_available": claude_cli_available,
                    "requested_timeout_ms": request.timeout_ms,
                })),
            }),
        }
    }

    fn execute(&self, request: ExecuteRequest) -> ProviderResult<ExecuteResponse> {
        let response = exec::execute_task(
            request.task.as_str(),
            request.input.as_deref(),
            request.timeout_ms,
        )?;

        Ok(ExecuteResponse {
            exit_code: 0,
            stdout: response.stdout,
            stderr: response.stderr,
            duration_ms: response.duration_ms,
        })
    }

    fn limits(&self, _request: LimitsRequest) -> ProviderResult<LimitsResponse> {
        let max_timeout_ms = config::ClaudeConfig::from_env()
            .ok()
            .map(|cfg| cfg.timeout_ms);
        let max_concurrency = config::max_concurrency().ok().or(Some(2));
        Ok(LimitsResponse {
            max_concurrency,
            max_timeout_ms,
            max_input_bytes: None,
        })
    }

    fn auth_state(&self, _request: AuthStateRequest) -> ProviderResult<AuthStateResponse> {
        if !config::api_key_configured() {
            return Ok(AuthStateResponse {
                state: AuthStateStatus::Unauthenticated,
                subject: None,
                scopes: Vec::new(),
                expires_at: None,
            });
        }

        Ok(AuthStateResponse {
            state: AuthStateStatus::Authenticated,
            subject: config::auth_subject(),
            scopes: config::auth_scopes(),
            expires_at: None,
        })
    }
}
