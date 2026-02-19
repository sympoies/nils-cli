use crate::client::ClaudeApiClient;
use crate::config;
use crate::prompts;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateResponse, AuthStateStatus, CapabilitiesRequest,
    CapabilitiesResponse, Capability, ExecuteRequest, ExecuteResponse, HealthStatus,
    HealthcheckRequest, HealthcheckResponse, LimitsRequest, LimitsResponse, ProviderError,
    ProviderErrorCategory, ProviderMaturity, ProviderMetadata, ProviderResult,
};
use serde_json::json;
use std::time::Instant;

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
        let prompt =
            prompts::render_execute_prompt(request.task.as_str(), request.input.as_deref());
        if prompt.trim().is_empty() {
            return Err(Box::new(ProviderError::new(
                ProviderErrorCategory::Validation,
                "missing-task",
                "execute task/input is required",
            )));
        }

        let config = config::ClaudeConfig::from_env()
            .map_err(|error| Box::new(config_error_to_provider_error(error)))?;
        let client =
            ClaudeApiClient::new(config).map_err(|error| Box::new(error.into_provider_error()))?;
        let started_at = Instant::now();
        let response = client
            .execute_prompt(prompt.as_str(), request.timeout_ms)
            .map_err(|error| Box::new((*error).into_provider_error()))?;

        let mut stderr = String::new();
        if let Some(request_id) = response.request_id.as_deref() {
            stderr = format!("request_id={request_id}");
        }

        Ok(ExecuteResponse {
            exit_code: 0,
            stdout: response.text,
            stderr,
            duration_ms: as_millis(started_at.elapsed()),
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

fn config_error_to_provider_error(error: config::ConfigError) -> ProviderError {
    let category = if error.code == "missing-api-key" {
        ProviderErrorCategory::Auth
    } else {
        ProviderErrorCategory::Validation
    };
    ProviderError::new(category, error.code, error.message).with_retryable(false)
}

fn as_millis(duration: std::time::Duration) -> Option<u64> {
    u64::try_from(duration.as_millis()).ok()
}
