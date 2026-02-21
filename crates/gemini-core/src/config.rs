use std::path::PathBuf;

use nils_common::env as shared_env;

use crate::paths;

pub const DEFAULT_MODEL: &str = "gemini-2.5-flash";
pub const DEFAULT_REASONING: &str = "medium";
pub const DEFAULT_STARSHIP_ENABLED: &str = "false";
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
    pub starship_enabled: String,
    pub auto_refresh_enabled: String,
    pub auto_refresh_min_days: String,
}

pub fn snapshot() -> RuntimeConfig {
    RuntimeConfig {
        model: shared_env::env_or_default("GEMINI_CLI_MODEL", DEFAULT_MODEL),
        reasoning: shared_env::env_or_default("GEMINI_CLI_REASONING", DEFAULT_REASONING),
        allow_dangerous_enabled_raw: std::env::var("GEMINI_ALLOW_DANGEROUS_ENABLED")
            .unwrap_or_default(),
        secret_dir: paths::resolve_secret_dir(),
        auth_file: paths::resolve_auth_file(),
        secret_cache_dir: paths::resolve_secret_cache_dir(),
        starship_enabled: shared_env::env_or_default(
            "GEMINI_STARSHIP_ENABLED",
            DEFAULT_STARSHIP_ENABLED,
        ),
        auto_refresh_enabled: shared_env::env_or_default(
            "GEMINI_AUTO_REFRESH_ENABLED",
            DEFAULT_AUTO_REFRESH_ENABLED,
        ),
        auto_refresh_min_days: shared_env::env_or_default(
            "GEMINI_AUTO_REFRESH_MIN_DAYS",
            DEFAULT_AUTO_REFRESH_MIN_DAYS,
        ),
    }
}
