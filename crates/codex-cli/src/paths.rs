use std::env;
use std::path::{Path, PathBuf};

pub fn resolve_secret_dir() -> Option<PathBuf> {
    if let Some(dir) = env_path("CODEX_SECRET_DIR") {
        return Some(dir);
    }

    let feature_dir = resolve_feature_dir()?;
    if feature_dir.join("init.zsh").is_file() || feature_dir.join("codex-tools.zsh").is_file() {
        return Some(feature_dir.join("secrets"));
    }
    Some(feature_dir)
}

pub fn resolve_auth_file() -> Option<PathBuf> {
    if let Some(path) = env_path("CODEX_AUTH_FILE") {
        return Some(path);
    }

    let home = home_dir()?;
    let primary = home.join(".codex").join("auth.json");
    let fallback = home.join(".codex").join("auth.json");

    let mut selected = primary.clone();
    if selected == primary && !primary.exists() && fallback.exists() {
        selected = fallback.clone();
    } else if selected == fallback && !fallback.exists() && primary.exists() {
        selected = primary.clone();
    }
    Some(selected)
}

pub fn resolve_secret_cache_dir() -> Option<PathBuf> {
    if let Some(path) = env_path("CODEX_SECRET_CACHE_DIR") {
        return Some(path);
    }

    let cache_root = if let Some(path) = env_path("ZSH_CACHE_DIR") {
        path
    } else {
        resolve_zdotdir()?.join("cache")
    };

    Some(cache_root.join("codex").join("secrets"))
}

pub fn resolve_feature_dir() -> Option<PathBuf> {
    let script_dir = resolve_script_dir()?;
    let feature_dir = script_dir.join("_features").join("codex");
    if feature_dir.is_dir() {
        Some(feature_dir)
    } else {
        None
    }
}

pub fn resolve_script_dir() -> Option<PathBuf> {
    if let Some(path) = env_path("ZSH_SCRIPT_DIR") {
        return Some(path);
    }
    Some(resolve_zdotdir()?.join("scripts"))
}

pub fn resolve_zdotdir() -> Option<PathBuf> {
    if let Some(path) = env_path("ZDOTDIR") {
        return Some(path);
    }

    if let Some(preload) = env_path("_ZSH_BOOTSTRAP_PRELOAD_PATH")
        && let Some(parent) = parent_dir(&preload, 2)
    {
        return Some(parent);
    }

    let home = home_dir()?;
    Some(home.join(".config").join("zsh"))
}

fn env_path(key: &str) -> Option<PathBuf> {
    let raw = env::var_os(key)?;
    if raw.is_empty() {
        return None;
    }
    Some(PathBuf::from(raw))
}

fn home_dir() -> Option<PathBuf> {
    env_path("HOME")
}

fn parent_dir(path: &Path, levels: usize) -> Option<PathBuf> {
    let mut current = path;
    for _ in 0..levels {
        current = current.parent()?;
    }
    Some(current.to_path_buf())
}
