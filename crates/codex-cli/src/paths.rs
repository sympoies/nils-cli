use std::path::{Path, PathBuf};

use crate::provider_profile::CODEX_PROVIDER_PROFILE;

pub fn resolve_secret_dir() -> Option<PathBuf> {
    crate::runtime::resolve_secret_dir()
}

pub fn resolve_secret_dir_from_env() -> Option<PathBuf> {
    nils_common::provider_runtime::paths::resolve_secret_dir_from_env(&CODEX_PROVIDER_PROFILE)
}

pub fn resolve_auth_file() -> Option<PathBuf> {
    crate::runtime::resolve_auth_file()
}

pub fn resolve_secret_cache_dir() -> Option<PathBuf> {
    crate::runtime::resolve_secret_cache_dir()
}

pub fn resolve_secret_timestamp_path(target_file: &Path) -> Option<PathBuf> {
    nils_common::provider_runtime::paths::resolve_secret_timestamp_path(
        &CODEX_PROVIDER_PROFILE,
        target_file,
    )
}

pub fn resolve_feature_dir() -> Option<PathBuf> {
    crate::runtime::resolve_feature_dir()
}

pub fn resolve_script_dir() -> Option<PathBuf> {
    crate::runtime::resolve_script_dir()
}

pub fn resolve_zdotdir() -> Option<PathBuf> {
    crate::runtime::resolve_zdotdir()
}
