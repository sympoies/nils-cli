use std::env;
use std::path::{Path, PathBuf};

pub fn resolve_secret_dir() -> Option<PathBuf> {
    if let Some(dir) = env_path("GEMINI_SECRET_DIR") {
        return Some(dir);
    }

    if let Some(home) = home_dir() {
        let modern = home.join(".gemini").join("secrets");
        let legacy = home.join(".config").join("gemini_secrets");
        if modern.exists() || !legacy.exists() {
            return Some(modern);
        }
        return Some(legacy);
    }

    let feature_dir = resolve_feature_dir()?;
    if feature_dir.join("init.zsh").is_file() || feature_dir.join("gemini-tools.zsh").is_file() {
        return Some(feature_dir.join("secrets"));
    }
    Some(feature_dir)
}

pub fn resolve_auth_file() -> Option<PathBuf> {
    if let Some(path) = env_path("GEMINI_AUTH_FILE") {
        return Some(path);
    }

    let home = home_dir()?;
    let modern = home.join(".gemini").join("oauth_creds.json");
    let legacy = home.join(".agents").join("auth.json");
    if modern.exists() || !legacy.exists() {
        Some(modern)
    } else {
        Some(legacy)
    }
}

pub fn resolve_secret_cache_dir() -> Option<PathBuf> {
    if let Some(path) = env_path("GEMINI_SECRET_CACHE_DIR") {
        return Some(path);
    }

    if let Some(path) = env_path("ZSH_CACHE_DIR") {
        return Some(path.join("gemini").join("secrets"));
    }

    if let Some(home) = home_dir() {
        return Some(home.join(".gemini").join("cache").join("secrets"));
    }

    Some(
        resolve_zdotdir()?
            .join("cache")
            .join("gemini")
            .join("secrets"),
    )
}

pub fn resolve_feature_dir() -> Option<PathBuf> {
    let script_dir = resolve_script_dir()?;
    let feature_dir = script_dir.join("_features").join("gemini");
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
