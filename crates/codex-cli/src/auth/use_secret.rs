use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::fs;
use crate::paths;

pub fn run(target: &str) -> Result<i32> {
    if target.is_empty() {
        eprintln!("codex-use: usage: codex-use <name|name.json|email>");
        return Ok(64);
    }

    if target.contains('/') || target.contains("..") {
        eprintln!("codex-use: invalid secret name: {target}");
        return Ok(64);
    }

    let secret_dir = match paths::resolve_secret_dir() {
        Some(dir) => dir,
        None => {
            eprintln!("codex-use: secret not found: {target}");
            return Ok(1);
        }
    };

    let is_email = target.contains('@');
    let mut secret_name = target.to_string();
    if !secret_name.ends_with(".json") && !is_email {
        secret_name.push_str(".json");
    }

    if secret_dir.join(&secret_name).is_file() {
        return apply_secret(&secret_dir, &secret_name);
    }

    match resolve_by_email(&secret_dir, target) {
        ResolveResult::Exact(name) => apply_secret(&secret_dir, &name),
        ResolveResult::Ambiguous { candidates } => {
            eprintln!("codex-use: identifier matches multiple secrets: {target}");
            eprintln!("codex-use: candidates: {}", candidates.join(", "));
            Ok(2)
        }
        ResolveResult::NotFound => {
            eprintln!("codex-use: secret not found: {target}");
            Ok(1)
        }
    }
}

fn apply_secret(secret_dir: &Path, secret_name: &str) -> Result<i32> {
    let source_file = secret_dir.join(secret_name);
    if !source_file.is_file() {
        eprintln!("codex: secret file {secret_name} not found");
        return Ok(1);
    }

    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok(1),
    };

    if auth_file.is_file() {
        let sync_result = crate::auth::sync::run()?;
        if sync_result != 0 {
            eprintln!("codex: failed to sync current auth before switching secrets");
            return Ok(1);
        }
    }

    let contents = std::fs::read(&source_file)?;
    fs::write_atomic(&auth_file, &contents, fs::SECRET_FILE_MODE)?;

    let iso = auth::last_refresh_from_auth_file(&auth_file).unwrap_or(None);
    let timestamp_path = secret_timestamp_path(&auth_file)?;
    fs::write_timestamp(&timestamp_path, iso.as_deref())?;

    println!("codex: applied {secret_name} to {}", auth_file.display());
    Ok(0)
}

fn resolve_by_email(secret_dir: &Path, target: &str) -> ResolveResult {
    let query = target.to_lowercase();
    let want_full = target.contains('@');

    let mut matches = Vec::new();
    if let Ok(entries) = std::fs::read_dir(secret_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let email = match auth::email_from_auth_file(&path) {
                Ok(Some(value)) => value,
                _ => continue,
            };
            let email_lower = email.to_lowercase();
            if want_full {
                if email_lower == query {
                    matches.push(file_name(&path));
                }
            } else if let Some(local_part) = email_lower.split('@').next() {
                if local_part == query {
                    matches.push(file_name(&path));
                }
            }
        }
    }

    if matches.len() == 1 {
        ResolveResult::Exact(matches.remove(0))
    } else if matches.is_empty() {
        ResolveResult::NotFound
    } else {
        ResolveResult::Ambiguous { candidates: matches }
    }
}

fn secret_timestamp_path(target_file: &PathBuf) -> Result<PathBuf> {
    let cache_dir = paths::resolve_secret_cache_dir()
        .ok_or_else(|| anyhow::anyhow!("CODEX_SECRET_CACHE_DIR not resolved"))?;
    let name = target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Ok(cache_dir.join(format!("{name}.timestamp")))
}

fn file_name(path: &PathBuf) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

enum ResolveResult {
    Exact(String),
    Ambiguous { candidates: Vec<String> },
    NotFound,
}
