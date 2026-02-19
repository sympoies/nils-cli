use std::path::PathBuf;

use crate::paths;

pub const DEFAULT_MODEL: &str = "gpt-5.1-codex-mini";
pub const DEFAULT_REASONING: &str = "medium";
pub const DEFAULT_AUTO_REFRESH_ENABLED: &str = "false";
pub const DEFAULT_AUTO_REFRESH_MIN_DAYS: &str = "5";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub model: String,
    pub reasoning: String,
    pub allow_dangerous_enabled_raw: String,
    pub secret_dir: Option<PathBuf>,
    pub auth_file: Option<PathBuf>,
    pub secret_cache_dir: Option<PathBuf>,
    pub auto_refresh_enabled: String,
    pub auto_refresh_min_days: String,
}

pub fn snapshot() -> RuntimeConfig {
    RuntimeConfig {
        model: env_or_default("CODEX_CLI_MODEL", DEFAULT_MODEL),
        reasoning: env_or_default("CODEX_CLI_REASONING", DEFAULT_REASONING),
        allow_dangerous_enabled_raw: std::env::var("CODEX_ALLOW_DANGEROUS_ENABLED")
            .unwrap_or_default(),
        secret_dir: paths::resolve_secret_dir(),
        auth_file: paths::resolve_auth_file(),
        secret_cache_dir: paths::resolve_secret_cache_dir(),
        auto_refresh_enabled: env_or_default(
            "CODEX_AUTO_REFRESH_ENABLED",
            DEFAULT_AUTO_REFRESH_ENABLED,
        ),
        auto_refresh_min_days: env_or_default(
            "CODEX_AUTO_REFRESH_MIN_DAYS",
            DEFAULT_AUTO_REFRESH_MIN_DAYS,
        ),
    }
}

pub fn env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
