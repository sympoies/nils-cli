use nils_common::process;
use reqwest::Url;

pub const API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
pub const BASE_URL_ENV: &str = "ANTHROPIC_BASE_URL";
pub const MODEL_ENV: &str = "CLAUDE_MODEL";
pub const FALLBACK_MODEL_ENV: &str = "ANTHROPIC_MODEL";
pub const API_VERSION_ENV: &str = "ANTHROPIC_API_VERSION";
pub const TIMEOUT_MS_ENV: &str = "CLAUDE_TIMEOUT_MS";
pub const MAX_TOKENS_ENV: &str = "CLAUDE_MAX_TOKENS";
pub const RETRY_MAX_ENV: &str = "CLAUDE_RETRY_MAX";
pub const MAX_CONCURRENCY_ENV: &str = "CLAUDE_MAX_CONCURRENCY";
pub const AUTH_SUBJECT_ENV: &str = "ANTHROPIC_AUTH_SUBJECT";
pub const AUTH_SCOPES_ENV: &str = "ANTHROPIC_AUTH_SCOPES";

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5-20250929";
pub const DEFAULT_API_VERSION: &str = "2023-06-01";
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_MAX_TOKENS: u32 = 1_024;
pub const DEFAULT_RETRY_MAX: u32 = 2;
pub const DEFAULT_MAX_CONCURRENCY: u32 = 2;

#[derive(Debug, Clone)]
pub struct ClaudeConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub api_version: String,
    pub timeout_ms: u64,
    pub max_tokens: u32,
    pub retry_max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeAuthState {
    pub api_key_configured: bool,
    pub subject: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub code: String,
    pub message: String,
}

impl ConfigError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ConfigError {}

impl ClaudeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let api_key = trim_env(API_KEY_ENV).ok_or_else(|| {
            ConfigError::new(
                "missing-api-key",
                format!("{API_KEY_ENV} is required for claude execute"),
            )
        })?;

        let base_url = parse_base_url_env()?;
        let model = resolve_model();
        let api_version =
            trim_env(API_VERSION_ENV).unwrap_or_else(|| DEFAULT_API_VERSION.to_string());
        let timeout_ms = parse_u64_env(TIMEOUT_MS_ENV, DEFAULT_TIMEOUT_MS)?;
        let max_tokens = parse_u32_env(MAX_TOKENS_ENV, DEFAULT_MAX_TOKENS)?;
        let retry_max = parse_u32_env(RETRY_MAX_ENV, DEFAULT_RETRY_MAX)?;

        Ok(Self {
            api_key,
            base_url,
            model,
            api_version,
            timeout_ms,
            max_tokens,
            retry_max,
        })
    }
}

pub fn auth_state() -> ClaudeAuthState {
    let api_key = trim_env(API_KEY_ENV);
    let subject = trim_env(AUTH_SUBJECT_ENV).or_else(|| api_key.as_deref().map(mask_api_key));
    let scopes = parse_scopes(trim_env(AUTH_SCOPES_ENV));

    ClaudeAuthState {
        api_key_configured: api_key.is_some(),
        subject,
        scopes,
    }
}

pub fn api_key_configured() -> bool {
    auth_state().api_key_configured
}

pub fn auth_subject() -> Option<String> {
    auth_state().subject
}

pub fn auth_scopes() -> Vec<String> {
    auth_state().scopes
}

pub fn max_concurrency() -> Result<u32, ConfigError> {
    parse_u32_env(MAX_CONCURRENCY_ENV, DEFAULT_MAX_CONCURRENCY)
}

pub fn claude_cli_available() -> bool {
    process::cmd_exists("claude")
}

fn parse_u64_env(key: &str, default: u64) -> Result<u64, ConfigError> {
    let Some(raw) = trim_env(key) else {
        return Ok(default);
    };

    raw.parse::<u64>().map_err(|_| {
        ConfigError::new(
            "invalid-config",
            format!("{key} must be an integer in milliseconds"),
        )
    })
}

fn parse_base_url_env() -> Result<String, ConfigError> {
    let raw = trim_env(BASE_URL_ENV).unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    normalize_base_url(raw.as_str()).map_err(|_| {
        ConfigError::new(
            "invalid-config",
            format!(
                "{BASE_URL_ENV} must be an absolute http(s) URL without query or fragment components"
            ),
        )
    })
}

fn normalize_base_url(raw: &str) -> Result<String, ()> {
    let parsed = Url::parse(raw).map_err(|_| ())?;
    if parsed.host_str().is_none() {
        return Err(());
    }
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(());
    }

    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn resolve_model() -> String {
    trim_env(MODEL_ENV)
        .or_else(|| trim_env(FALLBACK_MODEL_ENV))
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

fn parse_scopes(raw: Option<String>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default()
}

fn parse_u32_env(key: &str, default: u32) -> Result<u32, ConfigError> {
    let Some(raw) = trim_env(key) else {
        return Ok(default);
    };

    raw.parse::<u32>()
        .map_err(|_| ConfigError::new("invalid-config", format!("{key} must be an integer")))
}

fn trim_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn mask_api_key(key: &str) -> String {
    if key.chars().count() <= 4 {
        return "key:***".to_string();
    }

    let suffix = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("key:***{suffix}")
}
