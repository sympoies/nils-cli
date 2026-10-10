use nils_common::env as shared_env;
use nils_common::provider_usage::{ProviderUsageReason, classify_http_failure};
use reqwest::blocking::Client;
use std::fmt;
use std::time::Duration;

use super::backoff::{self, Backoff};

const DEFAULT_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const DEFAULT_ANTHROPIC_BETA: &str = "oauth-2025-04-20";
const DEFAULT_USER_AGENT: &str = "claude-code/2.1.0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsageFetchError {
    reason: ProviderUsageReason,
}

impl UsageFetchError {
    fn new(reason: ProviderUsageReason) -> Self {
        Self { reason }
    }

    pub const fn reason(&self) -> ProviderUsageReason {
        self.reason
    }
}

impl fmt::Display for UsageFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Claude usage request failed ({})",
            self.reason.as_str()
        )
    }
}

impl std::error::Error for UsageFetchError {}

/// Why a usage request produced no success body, before classification.
#[derive(Debug)]
pub(crate) enum RequestFailure {
    /// The HTTP client could not be built.
    Client,
    /// A previous caller recorded a rate limit for this token.
    Backoff,
    /// The request never produced a response.
    Transport { timeout: bool },
    /// The endpoint answered with a non-2xx status.
    Http { status: u16, body: String },
}

pub(crate) fn backoff_active(access_token: &str) -> bool {
    backoff::active_for(access_token)
}

pub fn fetch_usage(access_token: &str) -> Result<String, UsageFetchError> {
    let result = request_usage(access_token).map_err(|failure| {
        UsageFetchError::new(match failure {
            RequestFailure::Backoff => ProviderUsageReason::RateLimited,
            RequestFailure::Client => ProviderUsageReason::Unknown,
            RequestFailure::Transport { timeout: true } => ProviderUsageReason::Timeout,
            RequestFailure::Transport { timeout: false } => ProviderUsageReason::ServiceUnavailable,
            RequestFailure::Http { status, body } => classify_http_failure(status, &body),
        })
    });
    backoff::bind_prompt_token(access_token);
    result
}

/// Sends one `GET` to the OAuth usage endpoint with `access_token`.
pub(crate) fn request_usage(access_token: &str) -> Result<String, RequestFailure> {
    request_usage_with(access_token, None, DEFAULT_USER_AGENT)
}

/// Sends one `GET` to the OAuth usage endpoint with `query` appended and
/// `default_user_agent` unless `CLAUDE_PROMPT_SEGMENT_USER_AGENT` overrides it.
pub(crate) fn request_usage_with(
    access_token: &str,
    query: Option<&str>,
    default_user_agent: &str,
) -> Result<String, RequestFailure> {
    let cooldown = Backoff::acquire(access_token).ok();
    if cooldown
        .as_ref()
        .is_some_and(|state| state.active(backoff::now_epoch()))
        || (cooldown.is_none() && backoff::active_for(access_token))
    {
        return Err(RequestFailure::Backoff);
    }
    let endpoint = with_query(&usage_endpoint(), query);
    let max_time_seconds = env_u64("CLAUDE_PROMPT_SEGMENT_MAX_TIME_SECONDS", 5);
    let client = Client::builder()
        .timeout(Duration::from_secs(max_time_seconds))
        .build()
        .map_err(|_| RequestFailure::Client)?;

    let resp = client
        .get(&endpoint)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-beta", anthropic_beta())
        .header("User-Agent", user_agent(default_user_agent))
        .header("Accept", "application/json")
        .send()
        .map_err(|error| RequestFailure::Transport {
            timeout: error.is_timeout(),
        })?;

    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| retry_after_seconds(value, backoff::now_epoch()));
    let body = resp.text().unwrap_or_default();
    if status == 429
        || (status == 403
            && classify_http_failure(status, &body) == ProviderUsageReason::RateLimited)
    {
        if let Some(cooldown) = &cooldown {
            let _ = cooldown.record(backoff::now_epoch(), retry_after);
        }
        return Err(RequestFailure::Backoff);
    }
    if !(200..300).contains(&status) {
        return Err(RequestFailure::Http { status, body });
    }

    if let Some(cooldown) = &cooldown {
        let _ = cooldown.clear();
    }
    Ok(body)
}

fn retry_after_seconds(value: &str, now: u64) -> Option<u64> {
    value.trim().parse().ok().or_else(|| {
        let epoch = chrono::DateTime::parse_from_rfc2822(value.trim())
            .ok()?
            .timestamp();
        u64::try_from(epoch)
            .ok()
            .map(|epoch| epoch.saturating_sub(now))
    })
}

/// The OAuth usage endpoint: `CLAUDE_PROMPT_SEGMENT_ENDPOINT` or the default.
pub(crate) fn usage_endpoint() -> String {
    resolve_endpoint(shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_ENDPOINT"))
}

pub(crate) fn anthropic_beta() -> String {
    shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_ANTHROPIC_BETA")
        .unwrap_or_else(|| DEFAULT_ANTHROPIC_BETA.to_string())
}

/// `CLAUDE_PROMPT_SEGMENT_USER_AGENT`, else `default`.
pub(crate) fn user_agent(default: &str) -> String {
    shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_USER_AGENT")
        .unwrap_or_else(|| default.to_string())
}

/// Appends `query` to `endpoint`, after any query the endpoint already has.
fn with_query(endpoint: &str, query: Option<&str>) -> String {
    match query {
        None | Some("") => endpoint.to_string(),
        Some(query) if endpoint.ends_with('?') || endpoint.ends_with('&') => {
            format!("{endpoint}{query}")
        }
        Some(query) if endpoint.contains('?') => format!("{endpoint}&{query}"),
        Some(query) => format!("{endpoint}?{query}"),
    }
}

fn resolve_endpoint(configured: Option<String>) -> String {
    configured.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_ENDPOINT, resolve_endpoint, retry_after_seconds, with_query};
    use pretty_assertions::assert_eq;

    #[test]
    fn retry_after_parses_seconds_and_http_date_with_a_fake_clock() {
        assert_eq!(retry_after_seconds("0", 1000), Some(0));
        assert_eq!(retry_after_seconds("600", 1000), Some(600));
        assert_eq!(
            retry_after_seconds("Thu, 01 Jan 1970 00:30:00 GMT", 1000),
            Some(800)
        );
        assert_eq!(retry_after_seconds("invalid", 1000), None);
    }

    #[test]
    fn endpoint_resolver_preserves_an_explicit_override() {
        assert_eq!(
            resolve_endpoint(Some("http://127.0.0.1:9/usage".to_string())),
            "http://127.0.0.1:9/usage"
        );
    }

    #[test]
    fn status_query_is_appended_after_any_existing_query() {
        let query = Some("at_wall=1&skip_spend=1");
        assert_eq!(
            with_query("https://h/api/oauth/usage", query),
            "https://h/api/oauth/usage?at_wall=1&skip_spend=1"
        );
        assert_eq!(
            with_query("https://h/usage?x=1", query),
            "https://h/usage?x=1&at_wall=1&skip_spend=1"
        );
        assert_eq!(
            with_query("https://h/usage?", query),
            "https://h/usage?at_wall=1&skip_spend=1"
        );
        assert_eq!(with_query("https://h/usage", None), "https://h/usage");
    }

    #[test]
    fn endpoint_resolver_uses_the_production_default_when_unset() {
        assert_eq!(resolve_endpoint(None), DEFAULT_ENDPOINT);
        assert_eq!(
            DEFAULT_ENDPOINT,
            "https://api.anthropic.com/api/oauth/usage"
        );
    }
}
