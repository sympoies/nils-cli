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
        let readiness = AdapterReadiness::resolve();
        let execute_enabled = readiness.execute_available();
        let execute_description = readiness.execute_description();

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
        let readiness = AdapterReadiness::resolve();
        let mut details = json!({
            "maturity": "stable",
            "execute_available": readiness.execute_available(),
            "api_key_configured": readiness.api_key_configured(),
            "claude_cli_available": claude_cli_available,
            "requested_timeout_ms": request.timeout_ms,
            "readiness_reason_code": readiness.readiness_reason_code(),
            "readiness_reason": readiness.readiness_reason_message(),
        });

        if let Some(cfg) = readiness.config() {
            details["base_url"] = json!(cfg.base_url);
            details["model"] = json!(cfg.model);
            details["api_version"] = json!(cfg.api_version);
            details["timeout_ms"] = json!(cfg.timeout_ms);
        }

        if let Some(error) = readiness.config_error() {
            details["config_error_code"] = json!(error.code);
            details["config_error_message"] = json!(error.message);
        }

        Ok(HealthcheckResponse {
            status: readiness.health_status(),
            summary: Some(readiness.summary().to_string()),
            details: Some(details),
        })
    }

    fn execute(&self, request: ExecuteRequest) -> ProviderResult<ExecuteResponse> {
        let prompt =
            prompts::render_execute_prompt(request.task.as_str(), request.input.as_deref())
                .map_err(|error| Box::new(prompt_render_error_to_provider_error(error)))?;

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
        let max_concurrency = config::max_concurrency()
            .ok()
            .or(Some(config::DEFAULT_MAX_CONCURRENCY));
        Ok(LimitsResponse {
            max_concurrency,
            max_timeout_ms,
            max_input_bytes: None,
        })
    }

    fn auth_state(&self, _request: AuthStateRequest) -> ProviderResult<AuthStateResponse> {
        let auth_state = config::auth_state();
        if !auth_state.api_key_configured {
            return Ok(AuthStateResponse {
                state: AuthStateStatus::Unauthenticated,
                subject: None,
                scopes: Vec::new(),
                expires_at: None,
            });
        }

        Ok(AuthStateResponse {
            state: AuthStateStatus::Authenticated,
            subject: auth_state.subject,
            scopes: auth_state.scopes,
            expires_at: None,
        })
    }
}

#[derive(Debug, Clone)]
enum AdapterReadiness {
    Ready(config::ClaudeConfig),
    Degraded(config::ConfigError),
    Unhealthy {
        error: config::ConfigError,
        api_key_configured: bool,
    },
}

impl AdapterReadiness {
    fn resolve() -> Self {
        match config::ClaudeConfig::from_env() {
            Ok(cfg) => Self::Ready(cfg),
            Err(error) if error.code == "missing-api-key" => Self::Degraded(error),
            Err(error) => Self::Unhealthy {
                error,
                api_key_configured: config::api_key_configured(),
            },
        }
    }

    fn execute_available(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    fn execute_description(&self) -> String {
        match self {
            Self::Ready(_) => "claude execution is enabled".to_string(),
            Self::Degraded(_) => {
                format!("set {} to enable execute capability", config::API_KEY_ENV)
            }
            Self::Unhealthy { error, .. } => format!(
                "resolve claude configuration error ({}) to enable execute capability: {}",
                error.code, error.message
            ),
        }
    }

    fn health_status(&self) -> HealthStatus {
        match self {
            Self::Ready(_) => HealthStatus::Healthy,
            Self::Degraded(_) => HealthStatus::Degraded,
            Self::Unhealthy { .. } => HealthStatus::Unhealthy,
        }
    }

    fn summary(&self) -> &'static str {
        match self {
            Self::Ready(_) => "claude adapter is ready",
            Self::Degraded(_) => "claude adapter is partially ready",
            Self::Unhealthy { .. } => "claude adapter is unavailable",
        }
    }

    fn readiness_reason_code(&self) -> &str {
        match self {
            Self::Ready(_) => "ready",
            Self::Degraded(error) | Self::Unhealthy { error, .. } => error.code.as_str(),
        }
    }

    fn readiness_reason_message(&self) -> String {
        match self {
            Self::Ready(_) => "claude adapter is ready for execute".to_string(),
            Self::Degraded(error) | Self::Unhealthy { error, .. } => error.message.clone(),
        }
    }

    fn api_key_configured(&self) -> bool {
        match self {
            Self::Ready(_) => true,
            Self::Degraded(_) => false,
            Self::Unhealthy {
                api_key_configured, ..
            } => *api_key_configured,
        }
    }

    fn config(&self) -> Option<&config::ClaudeConfig> {
        match self {
            Self::Ready(cfg) => Some(cfg),
            Self::Degraded(_) | Self::Unhealthy { .. } => None,
        }
    }

    fn config_error(&self) -> Option<&config::ConfigError> {
        match self {
            Self::Ready(_) => None,
            Self::Degraded(error) | Self::Unhealthy { error, .. } => Some(error),
        }
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

fn prompt_render_error_to_provider_error(error: prompts::PromptRenderError) -> ProviderError {
    match error {
        prompts::PromptRenderError::MissingTask => ProviderError::new(
            ProviderErrorCategory::Validation,
            "missing-task",
            "execute task/input is required",
        ),
    }
}

fn as_millis(duration: std::time::Duration) -> Option<u64> {
    u64::try_from(duration.as_millis()).ok()
}
