use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

use crate::auth;
use crate::fs;
use crate::paths;

pub fn run() -> Result<i32> {
    if !is_configured() {
        return Ok(0);
    }

    let min_days_raw =
        std::env::var("CODEX_AUTO_REFRESH_MIN_DAYS").unwrap_or_else(|_| "5".to_string());
    let min_days = match min_days_raw.parse::<i64>() {
        Ok(value) => value,
        Err(_) => {
            eprintln!(
                "codex-auto-refresh: invalid CODEX_AUTO_REFRESH_MIN_DAYS: {}",
                min_days_raw
            );
            return Ok(64);
        }
    };

    let min_seconds = min_days.saturating_mul(86_400);
    let now_epoch = Utc::now().timestamp();

    let auth_file = paths::resolve_auth_file();
    if auth_file.is_some() {
        let sync_rc = auth::sync::run()?;
        if sync_rc != 0 {
            return Ok(1);
        }
    }

    let mut targets = Vec::new();
    if let Some(auth_file) = auth_file.as_ref() {
        targets.push(auth_file.clone());
    }
    if let Some(secret_dir) = paths::resolve_secret_dir() {
        if let Ok(entries) = std::fs::read_dir(&secret_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    targets.push(path);
                }
            }
        }
    }

    let mut refreshed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for target in targets {
        if !target.is_file() {
            if auth_file.as_ref().map(|p| p == &target).unwrap_or(false) {
                skipped += 1;
                continue;
            }
            eprintln!("codex-auto-refresh: missing file: {}", target.display());
            failed += 1;
            continue;
        }

        let timestamp_path = timestamp_path(&target)?;
        match should_refresh(&target, &timestamp_path, now_epoch, min_seconds) {
            RefreshDecision::Refresh => {
                let rc = if auth_file.as_ref().map(|p| p == &target).unwrap_or(false) {
                    auth::refresh::run(&[])?
                } else {
                    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    auth::refresh::run(&[name.to_string()])?
                };
                if rc == 0 {
                    refreshed += 1;
                } else {
                    failed += 1;
                }
            }
            RefreshDecision::Skip => {
                skipped += 1;
            }
            RefreshDecision::WarnFuture => {
                eprintln!(
                    "codex-auto-refresh: warning: future timestamp for {}",
                    target.display()
                );
                skipped += 1;
            }
        }
    }

    println!(
        "codex-auto-refresh: refreshed={} skipped={} failed={} (min_age_days={})",
        refreshed, skipped, failed, min_days
    );

    if failed > 0 {
        return Ok(1);
    }

    Ok(0)
}

fn is_configured() -> bool {
    let mut candidates = Vec::new();
    if let Some(auth_file) = paths::resolve_auth_file() {
        candidates.push(auth_file);
    }
    if let Some(secret_dir) = paths::resolve_secret_dir() {
        if let Ok(entries) = std::fs::read_dir(&secret_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    candidates.push(path);
                }
            }
        }
    }

    candidates.iter().any(|path| path.is_file())
}

enum RefreshDecision {
    Refresh,
    Skip,
    WarnFuture,
}

fn should_refresh(
    target: &Path,
    timestamp_path: &Path,
    now_epoch: i64,
    min_seconds: i64,
) -> RefreshDecision {
    if let Some(last_epoch) = last_refresh_epoch(target, timestamp_path) {
        let age = now_epoch - last_epoch;
        if age < 0 {
            return RefreshDecision::WarnFuture;
        }
        if age >= min_seconds {
            RefreshDecision::Refresh
        } else {
            RefreshDecision::Skip
        }
    } else {
        RefreshDecision::Refresh
    }
}

fn last_refresh_epoch(target: &Path, timestamp_path: &Path) -> Option<i64> {
    if let Ok(content) = std::fs::read_to_string(timestamp_path) {
        let iso = normalize_iso(&content);
        if let Some(epoch) = iso_to_epoch(&iso) {
            return Some(epoch);
        }
    }

    let iso = auth::last_refresh_from_auth_file(target).ok().flatten()?;
    let iso = normalize_iso(&iso);
    let epoch = iso_to_epoch(&iso)?;
    let _ = fs::write_timestamp(timestamp_path, Some(&iso));
    Some(epoch)
}

fn normalize_iso(raw: &str) -> String {
    let mut trimmed = raw
        .split(&['\n', '\r'][..])
        .next()
        .unwrap_or("")
        .to_string();
    if let Some(dot) = trimmed.find('.') {
        if trimmed.ends_with('Z') {
            trimmed.truncate(dot);
            trimmed.push('Z');
        }
    }
    trimmed
}

fn iso_to_epoch(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.timestamp())
}

fn timestamp_path(target: &Path) -> Result<PathBuf> {
    let cache_dir = paths::resolve_secret_cache_dir()
        .ok_or_else(|| anyhow::anyhow!("CODEX_SECRET_CACHE_DIR not resolved"))?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Ok(cache_dir.join(format!("{name}.timestamp")))
}
