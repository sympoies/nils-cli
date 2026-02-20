use std::path::{Path, PathBuf};

use crate::auth;
use crate::auth::output;

pub fn run() -> i32 {
    run_with_json(false)
}

pub fn run_with_json(output_json: bool) -> i32 {
    if !is_configured() {
        if output_json {
            let _ = output::emit_result(
                "auth auto-refresh",
                output::obj(vec![
                    ("refreshed", output::n(0)),
                    ("skipped", output::n(0)),
                    ("failed", output::n(0)),
                    ("min_age_days", output::n(0)),
                    ("targets", output::arr(Vec::new())),
                ]),
            );
        }
        return 0;
    }

    let min_days_raw =
        std::env::var("GEMINI_AUTO_REFRESH_MIN_DAYS").unwrap_or_else(|_| "5".to_string());
    let min_days = match min_days_raw.parse::<i64>() {
        Ok(value) => value,
        Err(_) => {
            if output_json {
                let _ = output::emit_error(
                    "auth auto-refresh",
                    "invalid-min-days",
                    format!(
                        "gemini-auto-refresh: invalid GEMINI_AUTO_REFRESH_MIN_DAYS: {}",
                        min_days_raw
                    ),
                    Some(output::obj(vec![("value", output::s(min_days_raw))])),
                );
            } else {
                eprintln!(
                    "gemini-auto-refresh: invalid GEMINI_AUTO_REFRESH_MIN_DAYS: {}",
                    min_days_raw
                );
            }
            return 64;
        }
    };

    let min_seconds = min_days.saturating_mul(86_400);
    let now_epoch = auth::now_epoch_seconds();

    let auth_file = gemini_core::paths::resolve_auth_file();
    if auth_file.is_some() {
        let sync_rc = auth::sync::run_with_json(false);
        if sync_rc != 0 {
            if output_json {
                let _ = output::emit_error(
                    "auth auto-refresh",
                    "sync-failed",
                    "gemini-auto-refresh: failed to sync auth and secrets before refresh",
                    None,
                );
            }
            return 1;
        }
    }

    let mut targets = Vec::new();
    if let Some(auth_file) = auth_file.as_ref() {
        targets.push(auth_file.clone());
    }
    if let Some(secret_dir) = gemini_core::paths::resolve_secret_dir()
        && let Ok(entries) = std::fs::read_dir(&secret_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                targets.push(path);
            }
        }
    }

    let mut refreshed: i64 = 0;
    let mut skipped: i64 = 0;
    let mut failed: i64 = 0;
    let mut target_results: Vec<output::JsonValue> = Vec::new();

    for target in targets {
        if !target.is_file() {
            if auth_file.as_ref().map(|p| p == &target).unwrap_or(false) {
                skipped += 1;
                target_results.push(target_result(&target, "skipped", Some("auth-file-missing")));
                continue;
            }
            if !output_json {
                eprintln!("gemini-auto-refresh: missing file: {}", target.display());
            }
            failed += 1;
            target_results.push(target_result(&target, "failed", Some("missing-file")));
            continue;
        }

        let timestamp_path = timestamp_path(&target);
        match should_refresh(&target, timestamp_path.as_deref(), now_epoch, min_seconds) {
            RefreshDecision::Refresh => {
                let rc = if auth_file.as_ref().map(|p| p == &target).unwrap_or(false) {
                    if output_json {
                        auth::refresh::run_silent(&[])
                    } else {
                        auth::refresh::run(&[])
                    }
                } else {
                    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if output_json {
                        auth::refresh::run_silent(&[name.to_string()])
                    } else {
                        auth::refresh::run(&[name.to_string()])
                    }
                };

                if rc == 0 {
                    refreshed += 1;
                    target_results.push(target_result(&target, "refreshed", None));
                } else {
                    failed += 1;
                    target_results.push(target_result(
                        &target,
                        "failed",
                        Some(&format!("refresh-exit-{rc}")),
                    ));
                }
            }
            RefreshDecision::Skip => {
                skipped += 1;
                target_results.push(target_result(&target, "skipped", Some("not-due")));
            }
            RefreshDecision::WarnFuture => {
                if !output_json {
                    eprintln!(
                        "gemini-auto-refresh: warning: future timestamp for {}",
                        target.display()
                    );
                }
                skipped += 1;
                target_results.push(target_result(&target, "skipped", Some("future-timestamp")));
            }
        }
    }

    if output_json {
        let _ = output::emit_result(
            "auth auto-refresh",
            output::obj(vec![
                ("refreshed", output::n(refreshed)),
                ("skipped", output::n(skipped)),
                ("failed", output::n(failed)),
                ("min_age_days", output::n(min_days)),
                ("targets", output::arr(target_results)),
            ]),
        );
    } else {
        println!(
            "gemini-auto-refresh: refreshed={} skipped={} failed={} (min_age_days={})",
            refreshed, skipped, failed, min_days
        );
    }

    if failed > 0 {
        return 1;
    }

    0
}

fn target_result(target: &Path, status: &str, reason: Option<&str>) -> output::JsonValue {
    let mut fields = vec![
        (
            "target_file".to_string(),
            output::s(target.display().to_string()),
        ),
        ("status".to_string(), output::s(status)),
    ];
    if let Some(reason) = reason {
        fields.push(("reason".to_string(), output::s(reason)));
    }
    output::obj_dynamic(fields)
}

fn is_configured() -> bool {
    let mut candidates = Vec::new();
    if let Some(auth_file) = gemini_core::paths::resolve_auth_file() {
        candidates.push(auth_file);
    }
    if let Some(secret_dir) = gemini_core::paths::resolve_secret_dir()
        && let Ok(entries) = std::fs::read_dir(&secret_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                candidates.push(path);
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
    timestamp_path: Option<&Path>,
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

fn last_refresh_epoch(target: &Path, timestamp_path: Option<&Path>) -> Option<i64> {
    if let Some(path) = timestamp_path
        && let Ok(content) = std::fs::read_to_string(path)
    {
        let iso = auth::normalize_iso(&content);
        if let Some(epoch) = auth::parse_rfc3339_epoch(&iso) {
            return Some(epoch);
        }
    }

    let iso = auth::last_refresh_from_auth_file(target).ok().flatten()?;
    let iso = auth::normalize_iso(&iso);
    let epoch = auth::parse_rfc3339_epoch(&iso)?;
    if let Some(path) = timestamp_path {
        let _ = auth::write_timestamp(path, Some(&iso));
    }
    Some(epoch)
}

fn timestamp_path(target: &Path) -> Option<PathBuf> {
    let cache_dir = gemini_core::paths::resolve_secret_cache_dir()?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Some(cache_dir.join(format!("{name}.timestamp")))
}
