use crate::config::ClaudeConfig;
use agent_runtime_core::schema::{ProviderError, ProviderErrorCategory};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::thread;
use std::time::Duration;

const USER_AGENT: &str = "nils-agent-provider-claude";

#[derive(Debug, Clone)]
pub struct ClaudeExecuteResult {
    pub text: String,
    pub request_id: Option<String>,
    pub response_json: Value,
}

#[derive(Debug, Clone)]
pub struct ClaudeClientError {
    pub category: ProviderErrorCategory,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub details: Option<Value>,
}

impl ClaudeClientError {
    pub fn into_provider_error(self) -> ProviderError {
        let mut error = ProviderError::new(self.category, self.code, self.message)
            .with_retryable(self.retryable);
        if let Some(details) = self.details {
            error = error.with_details(details);
        }
        error
    }
}

pub type ClaudeClientResult<T> = Result<T, Box<ClaudeClientError>>;

#[derive(Debug, Clone)]
pub struct ClaudeApiClient {
    http: reqwest::blocking::Client,
    config: ClaudeConfig,
}

impl ClaudeApiClient {
    pub fn new(config: ClaudeConfig) -> ClaudeClientResult<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|error| {
                Box::new(ClaudeClientError {
                    category: ProviderErrorCategory::Internal,
                    code: "client-init-failed".to_string(),
                    message: "failed to initialize claude http client".to_string(),
                    retryable: false,
                    details: Some(json!({ "error": error.to_string() })),
                })
            })?;
        Ok(Self { http, config })
    }

    pub fn execute_prompt(
        &self,
        prompt: &str,
        timeout_override_ms: Option<u64>,
    ) -> ClaudeClientResult<ClaudeExecuteResult> {
        let endpoint = format!("{}/v1/messages", self.config.base_url);
        let mut attempt = 0u32;

        loop {
            let response = self.send_once(endpoint.as_str(), prompt, timeout_override_ms);
            match response {
                Ok(success) => return Ok(success),
                Err(error) => {
                    if !error.retryable || attempt >= self.config.retry_max {
                        return Err(error);
                    }
                    let backoff_ms = 200u64.saturating_mul(2u64.saturating_pow(attempt));
                    thread::sleep(Duration::from_millis(backoff_ms.min(2_000)));
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }

    fn send_once(
        &self,
        endpoint: &str,
        prompt: &str,
        timeout_override_ms: Option<u64>,
    ) -> ClaudeClientResult<ClaudeExecuteResult> {
        let payload = json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "messages": [
                { "role": "user", "content": prompt }
            ]
        });

        let mut request = self
            .http
            .post(endpoint)
            .header("x-api-key", self.config.api_key.as_str())
            .header("anthropic-version", self.config.api_version.as_str())
            .header("content-type", "application/json")
            .header("user-agent", USER_AGENT)
            .json(&payload);

        if let Some(timeout_ms) = timeout_override_ms {
            request = request.timeout(Duration::from_millis(timeout_ms));
        }

        let response = request.send().map_err(|error| {
            let (category, code, retryable) = if error.is_timeout() {
                (ProviderErrorCategory::Timeout, "request-timeout", true)
            } else if error.is_connect() {
                (ProviderErrorCategory::Network, "network-error", true)
            } else {
                (ProviderErrorCategory::Network, "request-failed", true)
            };

            Box::new(ClaudeClientError {
                category,
                code: code.to_string(),
                message: format!("claude request failed: {error}"),
                retryable,
                details: Some(json!({
                    "endpoint": endpoint,
                    "error": error.to_string(),
                })),
            })
        })?;

        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let body = response.text().unwrap_or_default();
        if status.is_success() {
            return parse_success(body.as_str(), request_id);
        }

        Err(Box::new(parse_http_error(status, request_id, body)))
    }
}

fn parse_success(
    body: &str,
    request_id: Option<String>,
) -> ClaudeClientResult<ClaudeExecuteResult> {
    let response_json: Value = serde_json::from_str(body).map_err(|error| {
        Box::new(ClaudeClientError {
            category: ProviderErrorCategory::Internal,
            code: "invalid-json-response".to_string(),
            message: "claude api returned non-json response".to_string(),
            retryable: false,
            details: Some(json!({
                "error": error.to_string(),
                "body": body,
            })),
        })
    })?;

    let text = extract_text_blocks(&response_json)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| response_json.to_string());
    Ok(ClaudeExecuteResult {
        text,
        request_id,
        response_json,
    })
}

fn extract_text_blocks(value: &Value) -> Option<String> {
    let blocks = value.get("content")?.as_array()?;
    let parts = blocks
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                return block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            None
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(""))
}

fn parse_http_error(
    status: StatusCode,
    request_id: Option<String>,
    body: String,
) -> ClaudeClientError {
    let parsed_json = serde_json::from_str::<Value>(body.as_str()).ok();
    let error_message = parsed_json
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "claude api request failed".to_string());
    let error_type = parsed_json
        .as_ref()
        .and_then(|value| value.pointer("/error/type"))
        .and_then(Value::as_str)
        .unwrap_or("unknown_error");

    let (category, code, retryable) = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            (ProviderErrorCategory::Auth, "auth-failed", false)
        }
        StatusCode::TOO_MANY_REQUESTS => (ProviderErrorCategory::RateLimit, "rate-limited", true),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
            (ProviderErrorCategory::Timeout, "request-timeout", true)
        }
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY => {
            (ProviderErrorCategory::Validation, "invalid-request", false)
        }
        _ if status.is_server_error() => (
            ProviderErrorCategory::Unavailable,
            "upstream-unavailable",
            true,
        ),
        _ => (ProviderErrorCategory::Unknown, "api-error", false),
    };

    ClaudeClientError {
        category,
        code: code.to_string(),
        message: format!("claude api error ({}): {}", status.as_u16(), error_message),
        retryable,
        details: Some(json!({
            "status": status.as_u16(),
            "error_type": error_type,
            "request_id": request_id,
            "body": parsed_json.unwrap_or(Value::String(body)),
        })),
    }
}
