use std::path::PathBuf;

pub fn resolve_secret_dir() -> Option<PathBuf> {
    crate::runtime::resolve_secret_dir()
}

pub fn resolve_auth_file() -> Option<PathBuf> {
    crate::runtime::resolve_auth_file()
}

pub fn resolve_secret_cache_dir() -> Option<PathBuf> {
    crate::runtime::resolve_secret_cache_dir()
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
