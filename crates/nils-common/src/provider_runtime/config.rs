use std::path::PathBuf;

use crate::env as shared_env;

use super::paths;
use super::profile::ProviderProfile;

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

pub fn snapshot(profile: &ProviderProfile) -> RuntimeConfig {
    RuntimeConfig {
        model: shared_env::env_or_default(profile.env.model, profile.defaults.model),
        reasoning: shared_env::env_or_default(profile.env.reasoning, profile.defaults.reasoning),
        allow_dangerous_enabled_raw: std::env::var(profile.env.allow_dangerous_enabled)
            .unwrap_or_default(),
        secret_dir: paths::resolve_secret_dir(profile),
        auth_file: paths::resolve_auth_file(profile),
        secret_cache_dir: paths::resolve_secret_cache_dir(profile),
        starship_enabled: shared_env::env_or_default(
            profile.env.starship_enabled,
            profile.defaults.starship_enabled,
        ),
        auto_refresh_enabled: shared_env::env_or_default(
            profile.env.auto_refresh_enabled,
            profile.defaults.auto_refresh_enabled,
        ),
        auto_refresh_min_days: shared_env::env_or_default(
            profile.env.auto_refresh_min_days,
            profile.defaults.auto_refresh_min_days,
        ),
    }
}
