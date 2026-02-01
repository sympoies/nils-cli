use anyhow::Result;
use std::path::Path;

use crate::auth;
use crate::fs;
use crate::paths;

pub fn run() -> Result<i32> {
    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok(1),
    };

    if !auth_file.is_file() {
        eprintln!("codex: {} not found", auth_file.display());
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

    let secret_dir = paths::resolve_secret_dir();
    let mut matched: Option<(String, MatchMode)> = None;

    if let Some(secret_dir) = secret_dir {
        if let Ok(entries) = std::fs::read_dir(&secret_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }

                if let Some(key) = auth_key.as_deref() {
                    if let Ok(Some(candidate_key)) = auth::identity_key_from_auth_file(&path) {
                        if candidate_key == key {
                            let candidate_hash = match fs::sha256_file(&path) {
                                Ok(hash) => hash,
                                Err(_) => {
                                    eprintln!("codex: failed to hash {}", path.display());
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
                    }
                }

                let candidate_hash = match fs::sha256_file(&path) {
                    Ok(hash) => hash,
                    Err(_) => {
                        eprintln!("codex: failed to hash {}", path.display());
                        return Ok(1);
                    }
                };
                if candidate_hash == auth_hash {
                    matched = Some((file_name(&path), MatchMode::Exact));
                    break;
                }
            }
        }
    }

    if let Some((secret_name, mode)) = matched {
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
        return Ok(0);
    }

    println!(
        "codex: {} does not match any known secret",
        auth_file.display()
    );
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
