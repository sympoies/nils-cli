use claude_core::config;
use nils_common::shell::{SingleQuoteEscapeStyle, quote_posix_single_with_style};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ConfigEnvelope {
    schema_version: &'static str,
    command: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<ConfigResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ConfigErrorPayload>,
}

#[derive(Debug, Serialize)]
struct ConfigResult {
    api_key_configured: bool,
    base_url: String,
    model: String,
    api_version: String,
    timeout_ms: u64,
    max_tokens: u32,
    retry_max: u32,
    max_concurrency: u32,
}

#[derive(Debug, Serialize)]
struct ConfigErrorPayload {
    code: &'static str,
    message: String,
}

pub fn show(json: bool) -> i32 {
    let result = ConfigResult {
        api_key_configured: config::api_key_configured(),
        base_url: read_env_trimmed(config::BASE_URL_ENV)
            .unwrap_or_else(|| config::DEFAULT_BASE_URL.to_string()),
        model: read_env_trimmed(config::MODEL_ENV)
            .or_else(|| read_env_trimmed(config::FALLBACK_MODEL_ENV))
            .unwrap_or_else(|| config::DEFAULT_MODEL.to_string()),
        api_version: read_env_trimmed(config::API_VERSION_ENV)
            .unwrap_or_else(|| config::DEFAULT_API_VERSION.to_string()),
        timeout_ms: parse_u64(config::TIMEOUT_MS_ENV, config::DEFAULT_TIMEOUT_MS),
        max_tokens: parse_u32(config::MAX_TOKENS_ENV, config::DEFAULT_MAX_TOKENS),
        retry_max: parse_u32(config::RETRY_MAX_ENV, config::DEFAULT_RETRY_MAX),
        max_concurrency: parse_u32(config::MAX_CONCURRENCY_ENV, config::DEFAULT_MAX_CONCURRENCY),
    };

    if json {
        let payload = ConfigEnvelope {
            schema_version: "claude-cli.config.v1",
            command: "config show",
            ok: true,
            result: Some(result),
            error: None,
        };

        match serde_json::to_string_pretty(&payload) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(err) => {
                eprintln!("claude-cli config: failed to encode json: {err}");
                1
            }
        }
    } else {
        println!(
            "{}_configured={}",
            config::API_KEY_ENV,
            result.api_key_configured
        );
        println!("{}={}", config::BASE_URL_ENV, result.base_url);
        println!("{}={}", config::MODEL_ENV, result.model);
        println!("{}={}", config::API_VERSION_ENV, result.api_version);
        println!("{}={}", config::TIMEOUT_MS_ENV, result.timeout_ms);
        println!("{}={}", config::MAX_TOKENS_ENV, result.max_tokens);
        println!("{}={}", config::RETRY_MAX_ENV, result.retry_max);
        println!("{}={}", config::MAX_CONCURRENCY_ENV, result.max_concurrency);
        0
    }
}

pub fn set(key: &str, value: &str) -> i32 {
    let (env_key, normalized) = match key {
        "api-key" | "api_key" | "ANTHROPIC_API_KEY" => (config::API_KEY_ENV, value.to_string()),
        "base-url" | "base_url" | "ANTHROPIC_BASE_URL" => (
            config::BASE_URL_ENV,
            value.trim_end_matches('/').to_string(),
        ),
        "model" | "CLAUDE_MODEL" => (config::MODEL_ENV, value.to_string()),
        "api-version" | "api_version" | "ANTHROPIC_API_VERSION" => {
            (config::API_VERSION_ENV, value.to_string())
        }
        "timeout-ms" | "timeout_ms" | "CLAUDE_TIMEOUT_MS" => {
            if value.parse::<u64>().is_err() {
                eprintln!("claude-cli config: timeout-ms must be an integer");
                return 64;
            }
            (config::TIMEOUT_MS_ENV, value.to_string())
        }
        "max-tokens" | "max_tokens" | "CLAUDE_MAX_TOKENS" => {
            if value.parse::<u32>().is_err() {
                eprintln!("claude-cli config: max-tokens must be an integer");
                return 64;
            }
            (config::MAX_TOKENS_ENV, value.to_string())
        }
        "retry-max" | "retry_max" | "CLAUDE_RETRY_MAX" => {
            if value.parse::<u32>().is_err() {
                eprintln!("claude-cli config: retry-max must be an integer");
                return 64;
            }
            (config::RETRY_MAX_ENV, value.to_string())
        }
        "max-concurrency" | "max_concurrency" | "CLAUDE_MAX_CONCURRENCY" => {
            if value.parse::<u32>().is_err() {
                eprintln!("claude-cli config: max-concurrency must be an integer");
                return 64;
            }
            (config::MAX_CONCURRENCY_ENV, value.to_string())
        }
        _ => {
            eprintln!("claude-cli config: unknown key: {key}");
            eprintln!(
                "claude-cli config: keys: api-key|base-url|model|api-version|timeout-ms|max-tokens|retry-max|max-concurrency"
            );
            return 64;
        }
    };

    println!(
        "export {}={}",
        env_key,
        quote_posix_single_with_style(&normalized, SingleQuoteEscapeStyle::DoubleQuoteBoundary)
    );
    0
}

fn read_env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_u64(key: &str, default: u64) -> u64 {
    read_env_trimmed(key)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn parse_u32(key: &str, default: u32) -> u32 {
    read_env_trimmed(key)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}
