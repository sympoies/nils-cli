use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::fs as codex_fs;
use crate::paths;

pub struct CacheEntry {
    pub non_weekly_label: String,
    pub non_weekly_remaining: i64,
    pub non_weekly_reset_epoch: Option<i64>,
    pub weekly_remaining: i64,
    pub weekly_reset_epoch: i64,
}

pub fn clear_starship_cache() -> Result<()> {
    let root = cache_root().context("cache root")?;
    if !root.is_absolute() {
        anyhow::bail!(
            "codex-rate-limits: refusing to clear cache with non-absolute cache root: {}",
            root.display()
        );
    }
    if root == Path::new("/") {
        anyhow::bail!(
            "codex-rate-limits: refusing to clear cache with invalid cache root: {}",
            root.display()
        );
    }

    let cache_dir = root.join("codex").join("starship-rate-limits");
    let cache_dir_str = cache_dir.to_string_lossy();
    if !cache_dir_str.ends_with("/codex/starship-rate-limits") {
        anyhow::bail!(
            "codex-rate-limits: refusing to clear unexpected cache dir: {}",
            cache_dir.display()
        );
    }

    if cache_dir.is_dir() {
        fs::remove_dir_all(&cache_dir).ok();
    }

    Ok(())
}

pub fn cache_file_for_target(target_file: &Path) -> Result<PathBuf> {
    let cache_dir = starship_cache_dir().context("cache dir")?;

    if let Some(secret_dir) = paths::resolve_secret_dir() {
        if target_file.starts_with(&secret_dir) {
            let display = secret_file_basename(target_file)?;
            let key = cache_key(&display)?;
            return Ok(cache_dir.join(format!("{key}.kv")));
        }

        if let Some(secret_name) = secret_name_for_auth(target_file, &secret_dir) {
            let key = cache_key(&secret_name)?;
            return Ok(cache_dir.join(format!("{key}.kv")));
        }
    }

    let hash = codex_fs::sha256_file(target_file)?;
    Ok(cache_dir.join(format!("auth_{}.kv", hash.to_lowercase())))
}

pub fn secret_name_for_target(target_file: &Path) -> Option<String> {
    let secret_dir = paths::resolve_secret_dir()?;
    if target_file.starts_with(&secret_dir) {
        return secret_file_basename(target_file).ok();
    }
    secret_name_for_auth(target_file, &secret_dir)
}

pub fn read_cache_entry(target_file: &Path) -> Result<CacheEntry> {
    let cache_file = cache_file_for_target(target_file)?;
    if !cache_file.is_file() {
        anyhow::bail!(
            "codex-rate-limits: cache not found (run codex-rate-limits without --cached, or codex-starship, to populate): {}",
            cache_file.display()
        );
    }

    let content = fs::read_to_string(&cache_file)
        .with_context(|| format!("failed to read cache: {}", cache_file.display()))?;
    let mut non_weekly_label: Option<String> = None;
    let mut non_weekly_remaining: Option<i64> = None;
    let mut non_weekly_reset_epoch: Option<i64> = None;
    let mut weekly_remaining: Option<i64> = None;
    let mut weekly_reset_epoch: Option<i64> = None;

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("non_weekly_label=") {
            non_weekly_label = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("non_weekly_remaining=") {
            non_weekly_remaining = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("non_weekly_reset_epoch=") {
            non_weekly_reset_epoch = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("weekly_remaining=") {
            weekly_remaining = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("weekly_reset_epoch=") {
            weekly_reset_epoch = value.parse::<i64>().ok();
        }
    }

    let non_weekly_label = match non_weekly_label {
        Some(value) if !value.is_empty() => value,
        _ => anyhow::bail!(
            "codex-rate-limits: invalid cache (missing non-weekly data): {}",
            cache_file.display()
        ),
    };
    let non_weekly_remaining = match non_weekly_remaining {
        Some(value) => value,
        _ => anyhow::bail!(
            "codex-rate-limits: invalid cache (missing non-weekly data): {}",
            cache_file.display()
        ),
    };
    let weekly_remaining = match weekly_remaining {
        Some(value) => value,
        _ => anyhow::bail!(
            "codex-rate-limits: invalid cache (missing weekly data): {}",
            cache_file.display()
        ),
    };
    let weekly_reset_epoch = match weekly_reset_epoch {
        Some(value) => value,
        _ => anyhow::bail!(
            "codex-rate-limits: invalid cache (missing weekly data): {}",
            cache_file.display()
        ),
    };

    Ok(CacheEntry {
        non_weekly_label,
        non_weekly_remaining,
        non_weekly_reset_epoch,
        weekly_remaining,
        weekly_reset_epoch,
    })
}

pub fn write_starship_cache(
    target_file: &Path,
    fetched_at_epoch: i64,
    non_weekly_label: &str,
    non_weekly_remaining: i64,
    weekly_remaining: i64,
    weekly_reset_epoch: i64,
    non_weekly_reset_epoch: Option<i64>,
) -> Result<()> {
    let cache_file = cache_file_for_target(target_file)?;
    if let Some(parent) = cache_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut lines = Vec::new();
    lines.push(format!("fetched_at={fetched_at_epoch}"));
    lines.push(format!("non_weekly_label={non_weekly_label}"));
    lines.push(format!("non_weekly_remaining={non_weekly_remaining}"));
    if let Some(epoch) = non_weekly_reset_epoch {
        lines.push(format!("non_weekly_reset_epoch={epoch}"));
    }
    lines.push(format!("weekly_remaining={weekly_remaining}"));
    lines.push(format!("weekly_reset_epoch={weekly_reset_epoch}"));

    let data = lines.join("\n");
    codex_fs::write_atomic(&cache_file, data.as_bytes(), codex_fs::SECRET_FILE_MODE)?;
    Ok(())
}

fn starship_cache_dir() -> Result<PathBuf> {
    let root = cache_root().context("cache root")?;
    Ok(root.join("codex").join("starship-rate-limits"))
}

fn cache_root() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ZSH_CACHE_DIR") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let zdotdir = paths::resolve_zdotdir()?;
    Some(zdotdir.join("cache"))
}

fn secret_name_for_auth(auth_file: &Path, secret_dir: &Path) -> Option<String> {
    let auth_key = auth::identity_key_from_auth_file(auth_file)
        .ok()
        .flatten()?;
    let entries = std::fs::read_dir(secret_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let candidate_key = match auth::identity_key_from_auth_file(&path).ok().flatten() {
            Some(value) => value,
            None => continue,
        };
        if candidate_key == auth_key {
            return secret_file_basename(&path).ok();
        }
    }
    None
}

fn secret_file_basename(path: &Path) -> Result<String> {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let base = file.trim_end_matches(".json");
    Ok(base.to_string())
}

fn cache_key(name: &str) -> Result<String> {
    if name.is_empty() {
        anyhow::bail!("missing cache key name");
    }
    let mut key = String::new();
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch);
        } else {
            key.push('_');
        }
    }
    while key.starts_with('_') {
        key.remove(0);
    }
    while key.ends_with('_') {
        key.pop();
    }
    if key.is_empty() {
        anyhow::bail!("invalid cache key name");
    }
    Ok(key)
}
