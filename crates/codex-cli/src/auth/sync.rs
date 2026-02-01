use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::fs;
use crate::paths;

pub fn run() -> Result<i32> {
    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok(0),
    };

    if !auth_file.is_file() {
        return Ok(0);
    }

    let auth_key = match auth::identity_key_from_auth_file(&auth_file) {
        Ok(Some(key)) => key,
        _ => return Ok(0),
    };

    let auth_last_refresh = auth::last_refresh_from_auth_file(&auth_file).unwrap_or(None);
    let auth_hash = match fs::sha256_file(&auth_file) {
        Ok(hash) => hash,
        Err(_) => {
            eprintln!("codex: failed to hash {}", auth_file.display());
            return Ok(1);
        }
    };

    let secret_dir = paths::resolve_secret_dir();
    if let Some(secret_dir) = secret_dir {
        if let Ok(entries) = std::fs::read_dir(&secret_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let candidate_key = match auth::identity_key_from_auth_file(&path) {
                    Ok(Some(key)) => key,
                    _ => continue,
                };
                if candidate_key != auth_key {
                    continue;
                }

                let secret_hash = match fs::sha256_file(&path) {
                    Ok(hash) => hash,
                    Err(_) => {
                        eprintln!("codex: failed to hash {}", path.display());
                        return Ok(1);
                    }
                };
                if secret_hash == auth_hash {
                    continue;
                }

                let contents = std::fs::read(&auth_file)?;
                fs::write_atomic(&path, &contents, fs::SECRET_FILE_MODE)?;

                let timestamp_path = secret_timestamp_path(&path)?;
                fs::write_timestamp(&timestamp_path, auth_last_refresh.as_deref())?;
            }
        }
    }

    let auth_timestamp = secret_timestamp_path(&auth_file)?;
    fs::write_timestamp(&auth_timestamp, auth_last_refresh.as_deref())?;

    Ok(0)
}

fn secret_timestamp_path(target_file: &Path) -> Result<PathBuf> {
    let cache_dir = paths::resolve_secret_cache_dir()
        .ok_or_else(|| anyhow::anyhow!("CODEX_SECRET_CACHE_DIR not resolved"))?;
    let name = target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Ok(cache_dir.join(format!("{name}.timestamp")))
}
