use std::io::Write;
use std::path::PathBuf;

use nils_common::provider_runtime;

use crate::provider_profile::CODEX_PROVIDER_PROFILE;

pub use nils_common::provider_runtime::{
    CoreError, CoreErrorCategory, ProviderCategoryHint, auth, json, jwt,
};

pub fn config_snapshot() -> provider_runtime::config::RuntimeConfig {
    provider_runtime::config::snapshot(&CODEX_PROVIDER_PROFILE)
}

pub fn resolve_secret_dir() -> Option<PathBuf> {
    provider_runtime::paths::resolve_secret_dir(&CODEX_PROVIDER_PROFILE)
}

pub fn resolve_auth_file() -> Option<PathBuf> {
    provider_runtime::paths::resolve_auth_file(&CODEX_PROVIDER_PROFILE)
}

pub fn resolve_secret_cache_dir() -> Option<PathBuf> {
    provider_runtime::paths::resolve_secret_cache_dir(&CODEX_PROVIDER_PROFILE)
}

pub fn resolve_feature_dir() -> Option<PathBuf> {
    provider_runtime::paths::resolve_feature_dir(&CODEX_PROVIDER_PROFILE)
}

pub fn resolve_script_dir() -> Option<PathBuf> {
    provider_runtime::paths::resolve_script_dir()
}

pub fn resolve_zdotdir() -> Option<PathBuf> {
    provider_runtime::paths::resolve_zdotdir()
}

pub fn require_allow_dangerous(caller: Option<&str>, stderr: &mut impl Write) -> bool {
    provider_runtime::exec::require_allow_dangerous(&CODEX_PROVIDER_PROFILE, caller, stderr)
}

pub fn allow_dangerous_status(caller: Option<&str>) -> (bool, Option<String>) {
    provider_runtime::exec::allow_dangerous_status(&CODEX_PROVIDER_PROFILE, caller)
}

pub fn check_allow_dangerous(caller: Option<&str>) -> Result<(), CoreError> {
    provider_runtime::exec::check_allow_dangerous(&CODEX_PROVIDER_PROFILE, caller)
}

pub fn exec_dangerous(prompt: &str, caller: &str, stderr: &mut impl Write) -> i32 {
    provider_runtime::exec::exec_dangerous(&CODEX_PROVIDER_PROFILE, prompt, caller, stderr)
}
