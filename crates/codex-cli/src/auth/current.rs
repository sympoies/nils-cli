use anyhow::Result;
use serde_json::json;
use std::io::ErrorKind;
use std::path::Path;

use crate::auth;
use crate::auth::output::{self, AuthCurrentResult};
use crate::paths;
use nils_common::fs;

pub fn run() -> Result<i32> {
    run_with_json(false)
}

pub fn run_with_json(output_json: bool) -> Result<i32> {
    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            if output_json {
                output::emit_error(
                    "auth current",
                    "auth-file-not-configured",
                    "CODEX_AUTH_FILE is not configured",
                    None,
                )?;
            }
            return Ok(1);
        }
    };

    if !auth_file.is_file() {
        if output_json {
            output::emit_error(
                "auth current",
                "auth-file-not-found",
                format!("{} not found", auth_file.display()),
                Some(json!({
                    "auth_file": auth_file.display().to_string(),
                })),
            )?;
        } else {
            eprintln!("codex: {} not found", auth_file.display());
        }
        return Ok(1);
    }

    let auth_key = auth::identity_key_from_auth_file(&auth_file).ok().flatten();
    let auth_hash = match fs::sha256_file(&auth_file) {
        Ok(hash) => hash,
        Err(_) => {
            eprintln!("codex: failed to hash {}", auth_file.display());
            return Ok(1);
        }
    };

    let secret_dir = match paths::resolve_secret_dir() {
        Some(path) => path,
        None => {
            emit_secret_dir_error(
                output_json,
                "secret-dir-not-configured",
                "CODEX_SECRET_DIR is not configured".to_string(),
                None,
            )?;
            return Ok(1);
        }
    };
    let entries = match std::fs::read_dir(&secret_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            emit_secret_dir_error(
                output_json,
                "secret-dir-not-found",
                format!("{} not found", secret_dir.display()),
                Some(json!({
                    "secret_dir": secret_dir.display().to_string(),
                })),
            )?;
            return Ok(1);
        }
        Err(err) => {
            emit_secret_dir_error(
                output_json,
                "secret-dir-read-failed",
                format!("failed to read {}: {err}", secret_dir.display()),
                Some(json!({
                    "secret_dir": secret_dir.display().to_string(),
                    "error": err.to_string(),
                })),
            )?;
            return Ok(1);
        }
    };
    let mut matched: Option<(String, MatchMode)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        if let Some(key) = auth_key.as_deref()
            && let Ok(Some(candidate_key)) = auth::identity_key_from_auth_file(&path)
            && candidate_key == key
        {
            let candidate_hash = match fs::sha256_file(&path) {
                Ok(hash) => hash,
                Err(_) => {
                    if output_json {
                        output::emit_error(
                            "auth current",
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
            let mode = if candidate_hash == auth_hash {
                MatchMode::Exact
            } else {
                MatchMode::Identity
            };
            matched = Some((file_name(&path), mode));
            break;
        }

        let candidate_hash = match fs::sha256_file(&path) {
            Ok(hash) => hash,
            Err(_) => {
                if output_json {
                    output::emit_error(
                        "auth current",
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
        if candidate_hash == auth_hash {
            matched = Some((file_name(&path), MatchMode::Exact));
            break;
        }
    }

    if let Some((secret_name, mode)) = matched {
        if output_json {
            let match_mode = match mode {
                MatchMode::Exact => "exact",
                MatchMode::Identity => "identity",
            };
            output::emit_result(
                "auth current",
                AuthCurrentResult {
                    auth_file: auth_file.display().to_string(),
                    matched: true,
                    matched_secret: Some(secret_name),
                    match_mode: Some(match_mode.to_string()),
                },
            )?;
        } else {
            match mode {
                MatchMode::Exact => {
                    println!("codex: {} matches {}", auth_file.display(), secret_name);
                }
                MatchMode::Identity => {
                    println!(
                        "codex: {} matches {} (identity; secret differs)",
                        auth_file.display(),
                        secret_name
                    );
                }
            }
        }
        return Ok(0);
    }

    if output_json {
        output::emit_error(
            "auth current",
            "secret-not-matched",
            format!("{} does not match any known secret", auth_file.display()),
            Some(json!({
                "auth_file": auth_file.display().to_string(),
                "matched": false,
            })),
        )?;
    } else {
        println!(
            "codex: {} does not match any known secret",
            auth_file.display()
        );
    }
    Ok(2)
}

#[derive(Copy, Clone)]
enum MatchMode {
    Exact,
    Identity,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn emit_secret_dir_error(
    output_json: bool,
    code: &str,
    message: String,
    details: Option<serde_json::Value>,
) -> Result<()> {
    if output_json {
        output::emit_error("auth current", code, message, details)?;
    } else {
        eprintln!("codex: {message}");
    }
    Ok(())
}
