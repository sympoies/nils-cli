use crate::{Limits, Provider, ResearchError};
use serde_json::Value;
use std::{future::Future, pin::Pin, time::Duration};

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Value, ResearchError>> + Send + 'a>>;
/// A single provider request; implementations must not retry or change model/provider.
pub trait Transport: Send + Sync {
    fn post<'a>(
        &'a self,
        provider: Provider,
        body: Value,
        limits: &'a Limits,
    ) -> ProviderFuture<'a>;
}
#[derive(Default)]
pub struct ApiTransport;
impl Transport for ApiTransport {
    fn post<'a>(
        &'a self,
        provider: Provider,
        body: Value,
        limits: &'a Limits,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            let key = std::env::var(provider.key_name())
                .ok()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    ResearchError::new("missing-key", format!("{} is not set", provider.key_name()))
                })?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(limits.timeout_seconds))
                .build()
                .map_err(|_| {
                    ResearchError::new("provider-error", "Cannot create provider client")
                })?;
            let request = match provider {
                Provider::Openai => client
                    .post("https://api.openai.com/v1/responses")
                    .bearer_auth(key),
                Provider::Anthropic => client
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01"),
            };
            let mut response = request.json(&body).send().await.map_err(transport_error)?;
            if !response.status().is_success() {
                let request_id = response
                    .headers()
                    .get("x-request-id")
                    .or_else(|| response.headers().get("request-id"))
                    .and_then(|h| h.to_str().ok())
                    .filter(|s| {
                        s.len() <= 160
                            && s.bytes()
                                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                    })
                    .map(str::to_owned);
                return Err(ResearchError::status(
                    response.status().as_u16(),
                    request_id,
                ));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
                if chunk.len() > limits.max_bytes - bytes.len() {
                    return Err(ResearchError::new(
                        "provider-response-limit",
                        "Provider response exceeded byte limit; content is unavailable",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).map_err(|_| {
                ResearchError::new("provider-response", "Provider returned invalid JSON")
            })
        })
    }
}
fn transport_error(error: reqwest::Error) -> ResearchError {
    if error.is_timeout() {
        ResearchError::new(
            "timeout",
            "Provider deadline exceeded; billing may be uncertain",
        )
    } else {
        ResearchError::new(
            "provider-error",
            "Provider transport failed; billing may be uncertain",
        )
    }
}
