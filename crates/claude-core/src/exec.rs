use crate::client::ClaudeApiClient;
use crate::config::{self, ConfigError};
use crate::prompts;
use agent_runtime_core::schema::{ProviderError, ProviderErrorCategory};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ClaudeExecResult {
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: Option<u64>,
}

pub fn execute_task(
    task: &str,
    input: Option<&str>,
    timeout_ms: Option<u64>,
) -> Result<ClaudeExecResult, Box<ProviderError>> {
    let prompt = prompts::render_execute_prompt(task, input);
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
        .execute_prompt(prompt.as_str(), timeout_ms)
        .map_err(|error| Box::new((*error).into_provider_error()))?;

    let mut stderr = String::new();
    if let Some(request_id) = response.request_id.as_deref() {
        stderr = format!("request_id={request_id}");
    }

    Ok(ClaudeExecResult {
        stdout: response.text,
        stderr,
        duration_ms: as_millis(started_at.elapsed()),
    })
}

fn config_error_to_provider_error(error: ConfigError) -> ProviderError {
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
