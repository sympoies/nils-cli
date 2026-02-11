use anyhow::Result;
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::auth::output::{self, AuthSyncResult};
use crate::fs;
use crate::paths;

pub fn run() -> Result<i32> {
    run_with_json(false)
}

pub fn run_with_json(output_json: bool) -> Result<i32> {
    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            if output_json {
                output::emit_result(
                    "auth sync",
                    AuthSyncResult {
                        auth_file: String::new(),
                        synced: 0,
                        skipped: 0,
                        failed: 0,
                        updated_files: Vec::new(),
                    },
                )?;
            }
            return Ok(0);
        }
    };

    if !auth_file.is_file() {
        if output_json {
            output::emit_result(
                "auth sync",
                AuthSyncResult {
                    auth_file: auth_file.display().to_string(),
                    synced: 0,
                    skipped: 1,
                    failed: 0,
                    updated_files: Vec::new(),
                },
            )?;
        }
        return Ok(0);
    }

    let auth_key = match auth::identity_key_from_auth_file(&auth_file) {
        Ok(Some(key)) => key,
        _ => {
            if output_json {
                output::emit_result(
                    "auth sync",
                    AuthSyncResult {
                        auth_file: auth_file.display().to_string(),
                        synced: 0,
                        skipped: 1,
                        failed: 0,
                        updated_files: Vec::new(),
                    },
                )?;
            }
            return Ok(0);
        }
    };

    let auth_last_refresh = auth::last_refresh_from_auth_file(&auth_file).unwrap_or(None);
    let auth_hash = match fs::sha256_file(&auth_file) {
        Ok(hash) => hash,
        Err(_) => {
            if output_json {
                output::emit_error(
                    "auth sync",
                    "hash-failed",
                    format!("failed to hash {}", auth_file.display()),
                    Some(json!({
                        "path": auth_file.display().to_string(),
                    })),
                )?;
            } else {
                eprintln!("codex: failed to hash {}", auth_file.display());
            }
            return Ok(1);
        }
    };

    let mut synced = 0usize;
    let mut skipped = 0usize;
    let failed = 0usize;
    let mut updated_files: Vec<String> = Vec::new();

    let secret_dir = paths::resolve_secret_dir();
    if let Some(secret_dir) = secret_dir
        && let Ok(entries) = std::fs::read_dir(&secret_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let candidate_key = match auth::identity_key_from_auth_file(&path) {
                Ok(Some(key)) => key,
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            if candidate_key != auth_key {
                skipped += 1;
                continue;
            }

            let secret_hash = match fs::sha256_file(&path) {
                Ok(hash) => hash,
                Err(_) => {
                    if output_json {
                        output::emit_error(
                            "auth sync",
                            "hash-failed",
                            format!("failed to hash {}", path.display()),
                            Some(json!({
                                "path": path.display().to_string(),
                            })),
                        )?;
                    } else {
                        eprintln!("codex: failed to hash {}", path.display());
                    }
                    return Ok(1);
                }
            };
            if secret_hash == auth_hash {
                skipped += 1;
                continue;
            }

            let contents = std::fs::read(&auth_file)?;
            fs::write_atomic(&path, &contents, fs::SECRET_FILE_MODE)?;

            let timestamp_path = secret_timestamp_path(&path)?;
            fs::write_timestamp(&timestamp_path, auth_last_refresh.as_deref())?;
            synced += 1;
            updated_files.push(path.display().to_string());
        }
    }

    let auth_timestamp = secret_timestamp_path(&auth_file)?;
    fs::write_timestamp(&auth_timestamp, auth_last_refresh.as_deref())?;

    if output_json {
        output::emit_result(
            "auth sync",
            AuthSyncResult {
                auth_file: auth_file.display().to_string(),
                synced,
                skipped,
                failed,
                updated_files,
            },
        )?;
    }

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
