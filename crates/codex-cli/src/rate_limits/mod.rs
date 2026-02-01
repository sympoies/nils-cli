use anyhow::Result;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::auth;
use crate::rate_limits::client::{fetch_usage, UsageRequest};

pub mod ansi;
pub mod cache;
pub mod client;
pub mod render;
pub mod writeback;

#[derive(Clone, Debug)]
pub struct RateLimitsOptions {
    pub clear_cache: bool,
    pub debug: bool,
    pub cached: bool,
    pub no_refresh_auth: bool,
    pub json: bool,
    pub one_line: bool,
    pub all: bool,
    pub async_mode: bool,
    pub jobs: Option<String>,
    pub secret: Option<String>,
}

pub fn run(args: &RateLimitsOptions) -> Result<i32> {
    let cached_mode = args.cached;
    let mut one_line = args.one_line;
    let mut all_mode = args.all;
    let output_json = args.json;

    let mut debug_mode = args.debug;
    if !debug_mode {
        if let Ok(raw) = std::env::var("ZSH_DEBUG") {
            if raw.parse::<i64>().unwrap_or(0) >= 2 {
                debug_mode = true;
            }
        }
    }

    if args.async_mode {
        return run_async_mode(args, debug_mode);
    }

    if cached_mode {
        one_line = true;
        if output_json {
            eprintln!("codex-rate-limits: --json is not supported with --cached");
            return Ok(64);
        }
        if args.clear_cache {
            eprintln!("codex-rate-limits: -c is not compatible with --cached");
            return Ok(64);
        }
    }

    if output_json && one_line {
        eprintln!("codex-rate-limits: --one-line is not compatible with --json");
        return Ok(64);
    }

    if args.clear_cache {
        if let Err(err) = cache::clear_starship_cache() {
            eprintln!("{err}");
            return Ok(1);
        }
    }

    if !all_mode
        && !output_json
        && !cached_mode
        && args.secret.is_none()
        && env_truthy("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED")
    {
        all_mode = true;
    }

    if all_mode {
        if output_json {
            eprintln!("codex-rate-limits: --json is not supported with --all");
            return Ok(64);
        }
        if args.secret.is_some() {
            eprintln!("codex-rate-limits: usage: codex-rate-limits [-c] [-d] [--cached] [--no-refresh-auth] [--json] [--one-line] [--all] [secret.json]");
            return Ok(64);
        }
        return run_all_mode(args, cached_mode, debug_mode);
    }

    run_single_mode(args, cached_mode, one_line, output_json)
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| matches!(value.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

struct AsyncEvent {
    secret_name: String,
    line: Option<String>,
    rc: i32,
    err: String,
}

struct AsyncFetchResult {
    line: Option<String>,
    rc: i32,
    err: String,
}

fn run_async_mode(args: &RateLimitsOptions, debug_mode: bool) -> Result<i32> {
    if args.json {
        eprintln!("codex-rate-limits: --async does not support --json");
        return Ok(64);
    }
    if args.one_line {
        eprintln!("codex-rate-limits: --async does not support --one-line");
        return Ok(64);
    }
    if let Some(secret) = args.secret.as_deref() {
        eprintln!(
            "codex-rate-limits: --async does not accept positional args: {}",
            secret
        );
        eprintln!(
            "codex-rate-limits: hint: async always queries all secrets under CODEX_SECRET_DIR"
        );
        return Ok(64);
    }
    if args.clear_cache && args.cached {
        eprintln!("codex-rate-limits: --async: -c is not compatible with --cached");
        return Ok(64);
    }

    let jobs = args
        .jobs
        .as_deref()
        .and_then(|raw| raw.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .unwrap_or(5);

    if args.clear_cache {
        if let Err(err) = cache::clear_starship_cache() {
            eprintln!("{err}");
            return Ok(1);
        }
    }

    let secret_dir = crate::paths::resolve_secret_dir().unwrap_or_default();
    if !secret_dir.is_dir() {
        eprintln!(
            "codex-rate-limits-async: CODEX_SECRET_DIR not found: {}",
            secret_dir.display()
        );
        return Ok(1);
    }

    let mut secret_files: Vec<PathBuf> = std::fs::read_dir(&secret_dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();

    if secret_files.is_empty() {
        eprintln!(
            "codex-rate-limits-async: no secrets found in {}",
            secret_dir.display()
        );
        return Ok(1);
    }

    secret_files.sort();

    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();
    let mut index = 0usize;
    let total = secret_files.len();
    let worker_count = jobs.min(total);

    let spawn_worker = |path: PathBuf,
                        cached_mode: bool,
                        no_refresh_auth: bool,
                        tx: mpsc::Sender<AsyncEvent>|
     -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let secret_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string();
            let result = async_fetch_one_line(&path, cached_mode, no_refresh_auth, &secret_name);
            let _ = tx.send(AsyncEvent {
                secret_name,
                line: result.line,
                rc: result.rc,
                err: result.err,
            });
        })
    };

    while index < total && handles.len() < worker_count {
        let path = secret_files[index].clone();
        index += 1;
        handles.push(spawn_worker(
            path,
            args.cached,
            args.no_refresh_auth,
            tx.clone(),
        ));
    }

    let mut events: std::collections::HashMap<String, AsyncEvent> =
        std::collections::HashMap::new();
    while events.len() < total {
        let event = match rx.recv() {
            Ok(event) => event,
            Err(_) => break,
        };
        events.insert(event.secret_name.clone(), event);

        if index < total {
            let path = secret_files[index].clone();
            index += 1;
            handles.push(spawn_worker(
                path,
                args.cached,
                args.no_refresh_auth,
                tx.clone(),
            ));
        }
    }

    drop(tx);
    for handle in handles {
        let _ = handle.join();
    }

    println!("\n🚦 Codex rate limits for all accounts\n");

    let mut rc = 0;
    let mut rows: Vec<Row> = Vec::new();
    let mut window_labels = std::collections::HashSet::new();
    let mut stderr_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for secret_file in &secret_files {
        let secret_name = secret_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();

        let mut row = Row::empty(secret_name.trim_end_matches(".json").to_string());
        let event = events.get(&secret_name);
        if let Some(event) = event {
            if !event.err.is_empty() {
                stderr_map.insert(secret_name.clone(), event.err.clone());
            }
            if !args.cached && event.rc != 0 {
                rc = 1;
            }

            if let Some(line) = &event.line {
                if let Some(parsed) = parse_one_line_output(line) {
                    row.window_label = parsed.window_label.clone();
                    row.non_weekly_remaining = parsed.non_weekly_remaining;
                    row.weekly_remaining = parsed.weekly_remaining;
                    row.weekly_reset_iso = parsed.weekly_reset_iso.clone();

                    if args.cached {
                        if let Ok(cache_entry) = cache::read_cache_entry(secret_file) {
                            row.non_weekly_reset_epoch = cache_entry.non_weekly_reset_epoch;
                            row.weekly_reset_epoch = Some(cache_entry.weekly_reset_epoch);
                        }
                    } else {
                        let values = crate::json::read_json(secret_file).ok();
                        if let Some(values) = values {
                            row.non_weekly_reset_epoch = crate::json::string_at(
                                &values,
                                &["codex_rate_limits", "non_weekly_reset_at_epoch"],
                            )
                            .and_then(|v| v.parse::<i64>().ok());
                            row.weekly_reset_epoch = crate::json::string_at(
                                &values,
                                &["codex_rate_limits", "weekly_reset_at_epoch"],
                            )
                            .and_then(|v| v.parse::<i64>().ok());
                        }
                        if row.non_weekly_reset_epoch.is_none()
                            || row.weekly_reset_epoch.is_none()
                        {
                            if let Ok(cache_entry) = cache::read_cache_entry(secret_file) {
                                if row.non_weekly_reset_epoch.is_none() {
                                    row.non_weekly_reset_epoch = cache_entry.non_weekly_reset_epoch;
                                }
                                if row.weekly_reset_epoch.is_none() {
                                    row.weekly_reset_epoch = Some(cache_entry.weekly_reset_epoch);
                                }
                            }
                        }
                    }

                    window_labels.insert(row.window_label.clone());
                    rows.push(row);
                    continue;
                }
            }
        }

        if !args.cached {
            rc = 1;
        }
        rows.push(row);
    }

    let mut non_weekly_header = "Non-weekly".to_string();
    let multiple_labels = window_labels.len() != 1;
    if !multiple_labels {
        if let Some(label) = window_labels.iter().next() {
            non_weekly_header = label.clone();
        }
    }

    let now_epoch = Utc::now().timestamp();

    println!(
        "{:<15}  {:>8}  {:>7}  {:>8}  {:>7}  {:<11}",
        "Name", non_weekly_header, "Left", "Weekly", "Left", "Reset"
    );
    println!("-----------------------------------------------------------------------");

    rows.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    for row in rows {
        let display_non_weekly = if multiple_labels && !row.window_label.is_empty() {
            if row.non_weekly_remaining >= 0 {
                format!("{}:{}", row.window_label, row.non_weekly_remaining)
            } else {
                "-".to_string()
            }
        } else if row.non_weekly_remaining >= 0 {
            row.non_weekly_remaining.to_string()
        } else {
            "-".to_string()
        };

        let non_weekly_left = row
            .non_weekly_reset_epoch
            .and_then(|epoch| render::format_until_epoch_compact(epoch, now_epoch))
            .unwrap_or_else(|| "-".to_string());
        let weekly_left = row
            .weekly_reset_epoch
            .and_then(|epoch| render::format_until_epoch_compact(epoch, now_epoch))
            .unwrap_or_else(|| "-".to_string());
        let reset_display = row
            .weekly_reset_epoch
            .and_then(render::format_epoch_local_datetime)
            .unwrap_or_else(|| "-".to_string());

        let non_weekly_display = ansi::format_percent_cell(&display_non_weekly, 8, None);
        let weekly_display = if row.weekly_remaining >= 0 {
            ansi::format_percent_cell(&row.weekly_remaining.to_string(), 8, None)
        } else {
            ansi::format_percent_cell("-", 8, None)
        };

        println!(
            "{:<15}  {}  {:>7}  {}  {:>7}  {:<11}",
            row.name, non_weekly_display, non_weekly_left, weekly_display, weekly_left, reset_display
        );
    }

    if debug_mode {
        let mut printed = false;
        for secret_file in &secret_files {
            let secret_name = secret_file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_string();
            if let Some(err) = stderr_map.get(&secret_name) {
                if err.is_empty() {
                    continue;
                }
                if !printed {
                    printed = true;
                    eprintln!();
                    eprintln!("codex-rate-limits-async: per-account stderr (captured):");
                }
                eprintln!("---- {} ----", secret_name);
                eprintln!("{err}");
            }
        }
    }

    Ok(rc)
}

fn async_fetch_one_line(
    target_file: &Path,
    cached_mode: bool,
    no_refresh_auth: bool,
    secret_name: &str,
) -> AsyncFetchResult {
    if cached_mode {
        return fetch_one_line_cached(target_file);
    }

    let mut attempt = 1;
    let max_attempts = 2;
    let mut network_err: Option<String> = None;

    let mut result = fetch_one_line_network(target_file, no_refresh_auth);
    if !result.err.is_empty() {
        network_err = Some(result.err.clone());
    }

    while attempt < max_attempts && result.rc == 3 {
        thread::sleep(Duration::from_millis(250));
        let next = fetch_one_line_network(target_file, no_refresh_auth);
        if !next.err.is_empty() {
            network_err = Some(next.err.clone());
        }
        result = next;
        attempt += 1;
        if result.rc != 3 {
            break;
        }
    }

    let mut errors: Vec<String> = Vec::new();
    if let Some(err) = network_err {
        errors.push(err);
    }

    let missing_line = result
        .line
        .as_ref()
        .map(|line| line.trim().is_empty())
        .unwrap_or(true);

    if result.rc != 0 || missing_line {
        let cached = fetch_one_line_cached(target_file);
        if !cached.err.is_empty() {
            errors.push(cached.err.clone());
        }
        if cached.rc == 0
            && cached
                .line
                .as_ref()
                .map(|line| !line.trim().is_empty())
                .unwrap_or(false)
        {
            if result.rc != 0 {
                errors.push(format!(
                    "codex-rate-limits-async: falling back to cache for {} (rc={})",
                    secret_name, result.rc
                ));
            }
            result = AsyncFetchResult {
                line: cached.line,
                rc: 0,
                err: String::new(),
            };
        }
    }

    let line = result.line.map(normalize_one_line);
    let err = errors.join("\n");
    AsyncFetchResult { line, rc: result.rc, err }
}

fn fetch_one_line_network(target_file: &Path, no_refresh_auth: bool) -> AsyncFetchResult {
    if !target_file.is_file() {
        return AsyncFetchResult {
            line: None,
            rc: 1,
            err: format!("codex-rate-limits: {} not found", target_file.display()),
        };
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);

    let usage_request = UsageRequest {
        target_file: target_file.to_path_buf(),
        refresh_on_401: !no_refresh_auth,
        base_url,
        connect_timeout_seconds: connect_timeout,
        max_time_seconds: max_time,
    };

    let usage = match fetch_usage(&usage_request) {
        Ok(value) => value,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("missing access_token") {
                return AsyncFetchResult {
                    line: None,
                    rc: 2,
                    err: format!(
                        "codex-rate-limits: missing access_token in {}",
                        target_file.display()
                    ),
                };
            }
            return AsyncFetchResult {
                line: None,
                rc: 3,
                err: msg,
            };
        }
    };

    if let Err(err) = writeback::write_weekly(target_file, &usage.json) {
        return AsyncFetchResult {
            line: None,
            rc: 4,
            err: err.to_string(),
        };
    }

    if is_auth_file(target_file) {
        match sync_auth_silent() {
            Ok((sync_rc, sync_err)) => {
                if sync_rc != 0 {
                    return AsyncFetchResult {
                        line: None,
                        rc: 5,
                        err: sync_err.unwrap_or_default(),
                    };
                }
            }
            Err(_) => {
                return AsyncFetchResult {
                    line: None,
                    rc: 1,
                    err: String::new(),
                };
            }
        }
    }

    let usage_data = match render::parse_usage(&usage.json) {
        Some(value) => value,
        None => {
            return AsyncFetchResult {
                line: None,
                rc: 3,
                err: "codex-rate-limits: invalid usage payload".to_string(),
            };
        }
    };

    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);

    let fetched_at_epoch = Utc::now().timestamp();
    if fetched_at_epoch > 0 {
        let _ = cache::write_starship_cache(
            target_file,
            fetched_at_epoch,
            &weekly.non_weekly_label,
            weekly.non_weekly_remaining,
            weekly.weekly_remaining,
            weekly.weekly_reset_epoch,
            weekly.non_weekly_reset_epoch,
        );
    }

    AsyncFetchResult {
        line: Some(format_one_line_output(
            target_file,
            &weekly.non_weekly_label,
            weekly.non_weekly_remaining,
            weekly.weekly_remaining,
            weekly.weekly_reset_epoch,
        )),
        rc: 0,
        err: String::new(),
    }
}

fn fetch_one_line_cached(target_file: &Path) -> AsyncFetchResult {
    match cache::read_cache_entry(target_file) {
        Ok(entry) => AsyncFetchResult {
            line: Some(format_one_line_output(
                target_file,
                &entry.non_weekly_label,
                entry.non_weekly_remaining,
                entry.weekly_remaining,
                entry.weekly_reset_epoch,
            )),
            rc: 0,
            err: String::new(),
        },
        Err(err) => AsyncFetchResult {
            line: None,
            rc: 1,
            err: err.to_string(),
        },
    }
}

fn format_one_line_output(
    target_file: &Path,
    non_weekly_label: &str,
    non_weekly_remaining: i64,
    weekly_remaining: i64,
    weekly_reset_epoch: i64,
) -> String {
    let prefix = cache::secret_name_for_target(target_file)
        .map(|name| format!("{name} "))
        .unwrap_or_default();
    let weekly_reset_iso = render::format_epoch_local_datetime(weekly_reset_epoch)
        .unwrap_or_else(|| "?".to_string());

    format!(
        "{}{}:{}% W:{}% {}",
        prefix, non_weekly_label, non_weekly_remaining, weekly_remaining, weekly_reset_iso
    )
}

fn normalize_one_line(line: String) -> String {
    line.replace('\n', " ").replace('\r', " ").replace('\t', " ")
}

fn sync_auth_silent() -> Result<(i32, Option<String>)> {
    let auth_file = match crate::paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok((0, None)),
    };

    if !auth_file.is_file() {
        return Ok((0, None));
    }

    let auth_key = match auth::identity_key_from_auth_file(&auth_file) {
        Ok(Some(key)) => key,
        _ => return Ok((0, None)),
    };

    let auth_last_refresh = auth::last_refresh_from_auth_file(&auth_file).unwrap_or(None);
    let auth_hash = match crate::fs::sha256_file(&auth_file) {
        Ok(hash) => hash,
        Err(_) => {
            return Ok((
                1,
                Some(format!("codex: failed to hash {}", auth_file.display())),
            ))
        }
    };

    if let Some(secret_dir) = crate::paths::resolve_secret_dir() {
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

                let secret_hash = match crate::fs::sha256_file(&path) {
                    Ok(hash) => hash,
                    Err(_) => {
                        return Ok((
                            1,
                            Some(format!("codex: failed to hash {}", path.display())),
                        ))
                    }
                };
                if secret_hash == auth_hash {
                    continue;
                }

                let contents = std::fs::read(&auth_file)?;
                crate::fs::write_atomic(&path, &contents, crate::fs::SECRET_FILE_MODE)?;

                let timestamp_path = secret_timestamp_path(&path)?;
                crate::fs::write_timestamp(&timestamp_path, auth_last_refresh.as_deref())?;
            }
        }
    }

    let auth_timestamp = secret_timestamp_path(&auth_file)?;
    crate::fs::write_timestamp(&auth_timestamp, auth_last_refresh.as_deref())?;

    Ok((0, None))
}

fn secret_timestamp_path(target_file: &Path) -> Result<PathBuf> {
    let cache_dir = crate::paths::resolve_secret_cache_dir()
        .ok_or_else(|| anyhow::anyhow!("CODEX_SECRET_CACHE_DIR not resolved"))?;
    let name = target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Ok(cache_dir.join(format!("{name}.timestamp")))
}

fn run_all_mode(args: &RateLimitsOptions, cached_mode: bool, debug_mode: bool) -> Result<i32> {
    let secret_dir = crate::paths::resolve_secret_dir().unwrap_or_default();
    if !secret_dir.is_dir() {
        eprintln!(
            "codex-rate-limits: CODEX_SECRET_DIR not found: {}",
            secret_dir.display()
        );
        return Ok(1);
    }

    let mut secret_files: Vec<PathBuf> = std::fs::read_dir(&secret_dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();

    if secret_files.is_empty() {
        eprintln!(
            "codex-rate-limits: no secrets found in {}",
            secret_dir.display()
        );
        return Ok(1);
    }

    secret_files.sort();

    println!("\n🚦 Codex rate limits for all accounts\n");

    let mut rc = 0;
    let mut rows: Vec<Row> = Vec::new();
    let mut window_labels = std::collections::HashSet::new();

    for secret_file in secret_files {
        let secret_name = secret_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();

        let mut row = Row::empty(secret_name.trim_end_matches(".json").to_string());
        let output = match single_one_line(
            &secret_file,
            cached_mode,
            args.no_refresh_auth,
            debug_mode,
        ) {
            Ok(Some(line)) => line,
            Ok(None) => String::new(),
            Err(_) => String::new(),
        };

        if output.is_empty() {
            if !cached_mode {
                rc = 1;
            }
            rows.push(row);
            continue;
        }

        if let Some(parsed) = parse_one_line_output(&output) {
            row.window_label = parsed.window_label.clone();
            row.non_weekly_remaining = parsed.non_weekly_remaining;
            row.weekly_remaining = parsed.weekly_remaining;
            row.weekly_reset_iso = parsed.weekly_reset_iso.clone();

            if cached_mode {
                if let Ok(cache_entry) = cache::read_cache_entry(&secret_file) {
                    row.non_weekly_reset_epoch = cache_entry.non_weekly_reset_epoch;
                    row.weekly_reset_epoch = Some(cache_entry.weekly_reset_epoch);
                }
            } else {
                let values = crate::json::read_json(&secret_file).ok();
                if let Some(values) = values {
                    row.non_weekly_reset_epoch = crate::json::string_at(
                        &values,
                        &["codex_rate_limits", "non_weekly_reset_at_epoch"],
                    )
                    .and_then(|v| v.parse::<i64>().ok());
                    row.weekly_reset_epoch = crate::json::string_at(
                        &values,
                        &["codex_rate_limits", "weekly_reset_at_epoch"],
                    )
                    .and_then(|v| v.parse::<i64>().ok());
                }
            }

            window_labels.insert(row.window_label.clone());
            rows.push(row);
        } else {
            if !cached_mode {
                rc = 1;
            }
            rows.push(row);
        }

    }

    let mut non_weekly_header = "Non-weekly".to_string();
    let multiple_labels = window_labels.len() != 1;
    if !multiple_labels {
        if let Some(label) = window_labels.iter().next() {
            non_weekly_header = label.clone();
        }
    }

    let now_epoch = Utc::now().timestamp();

    println!(
        "{:<15}  {:>8}  {:>7}  {:>8}  {:>7}  {:<11}",
        "Name", non_weekly_header, "Left", "Weekly", "Left", "Reset"
    );
    println!("-----------------------------------------------------------------------");

    rows.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    for row in rows {
        let display_non_weekly = if multiple_labels && !row.window_label.is_empty() {
            if row.non_weekly_remaining >= 0 {
                format!("{}:{}", row.window_label, row.non_weekly_remaining)
            } else {
                "-".to_string()
            }
        } else if row.non_weekly_remaining >= 0 {
            row.non_weekly_remaining.to_string()
        } else {
            "-".to_string()
        };

        let non_weekly_left = row
            .non_weekly_reset_epoch
            .and_then(|epoch| render::format_until_epoch_compact(epoch, now_epoch))
            .unwrap_or_else(|| "-".to_string());
        let weekly_left = row
            .weekly_reset_epoch
            .and_then(|epoch| render::format_until_epoch_compact(epoch, now_epoch))
            .unwrap_or_else(|| "-".to_string());
        let reset_display = row
            .weekly_reset_epoch
            .and_then(render::format_epoch_local_datetime)
            .unwrap_or_else(|| "-".to_string());

        let non_weekly_display = ansi::format_percent_cell(&display_non_weekly, 8, None);
        let weekly_display = if row.weekly_remaining >= 0 {
            ansi::format_percent_cell(&row.weekly_remaining.to_string(), 8, None)
        } else {
            ansi::format_percent_cell("-", 8, None)
        };

        println!(
            "{:<15}  {}  {:>7}  {}  {:>7}  {:<11}",
            row.name, non_weekly_display, non_weekly_left, weekly_display, weekly_left, reset_display
        );
    }

    Ok(rc)
}

fn run_single_mode(
    args: &RateLimitsOptions,
    cached_mode: bool,
    one_line: bool,
    output_json: bool,
) -> Result<i32> {
    let target_file = match resolve_target(args.secret.as_deref()) {
        Ok(path) => path,
        Err(code) => return Ok(code),
    };

    if !target_file.is_file() {
        eprintln!("codex-rate-limits: {} not found", target_file.display());
        return Ok(1);
    }

    if cached_mode {
        match cache::read_cache_entry(&target_file) {
            Ok(entry) => {
                let weekly_reset_iso = render::format_epoch_local_datetime(entry.weekly_reset_epoch)
                    .unwrap_or_else(|| "?".to_string());
                let prefix = cache::secret_name_for_target(&target_file)
                    .map(|name| format!("{name} "))
                    .unwrap_or_default();
                println!(
                    "{}{}:{}% W:{}% {}",
                    prefix,
                    entry.non_weekly_label,
                    entry.non_weekly_remaining,
                    entry.weekly_remaining,
                    weekly_reset_iso
                );
                return Ok(0);
            }
            Err(err) => {
                eprintln!("{err}");
                return Ok(1);
            }
        }
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);

    let usage_request = UsageRequest {
        target_file: target_file.clone(),
        refresh_on_401: !args.no_refresh_auth,
        base_url,
        connect_timeout_seconds: connect_timeout,
        max_time_seconds: max_time,
    };

    let usage = match fetch_usage(&usage_request) {
        Ok(value) => value,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("missing access_token") {
                eprintln!("codex-rate-limits: missing access_token in {}", target_file.display());
                return Ok(2);
            }
            eprintln!("{msg}");
            return Ok(3);
        }
    };

    if let Err(err) = writeback::write_weekly(&target_file, &usage.json) {
        eprintln!("{err}");
        return Ok(4);
    }

    if is_auth_file(&target_file) {
        let sync_rc = auth::sync::run()?;
        if sync_rc != 0 {
            return Ok(5);
        }
    }

    if output_json {
        println!("{}", usage.body);
        return Ok(0);
    }

    let usage_data = match render::parse_usage(&usage.json) {
        Some(value) => value,
        None => {
            eprintln!("codex-rate-limits: invalid usage payload");
            return Ok(3);
        }
    };

    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);

    let fetched_at_epoch = Utc::now().timestamp();
    if fetched_at_epoch > 0 {
        let _ = cache::write_starship_cache(
            &target_file,
            fetched_at_epoch,
            &weekly.non_weekly_label,
            weekly.non_weekly_remaining,
            weekly.weekly_remaining,
            weekly.weekly_reset_epoch,
            weekly.non_weekly_reset_epoch,
        );
    }

    if one_line {
        let prefix = cache::secret_name_for_target(&target_file)
            .map(|name| format!("{name} "))
            .unwrap_or_default();
        let weekly_reset_iso = render::format_epoch_local_datetime(weekly.weekly_reset_epoch)
            .unwrap_or_else(|| "?".to_string());

        println!(
            "{}{}:{}% W:{}% {}",
            prefix, weekly.non_weekly_label, weekly.non_weekly_remaining, weekly.weekly_remaining, weekly_reset_iso
        );
        return Ok(0);
    }

    println!("Rate limits remaining");
    let primary_reset = render::format_epoch_local_datetime(values.primary_reset_epoch)
        .unwrap_or_else(|| "?".to_string());
    let secondary_reset = render::format_epoch_local_datetime(values.secondary_reset_epoch)
        .unwrap_or_else(|| "?".to_string());

    println!(
        "{} {}% • {}",
        values.primary_label, values.primary_remaining, primary_reset
    );
    println!(
        "{} {}% • {}",
        values.secondary_label, values.secondary_remaining, secondary_reset
    );

    Ok(0)
}

fn single_one_line(
    target_file: &Path,
    cached_mode: bool,
    no_refresh_auth: bool,
    debug_mode: bool,
) -> Result<Option<String>> {
    if !target_file.is_file() {
        if debug_mode {
            eprintln!("codex-rate-limits: {} not found", target_file.display());
        }
        return Ok(None);
    }

    if cached_mode {
        return match cache::read_cache_entry(target_file) {
            Ok(entry) => {
                let weekly_reset_iso = render::format_epoch_local_datetime(entry.weekly_reset_epoch)
                    .unwrap_or_else(|| "?".to_string());
                let prefix = cache::secret_name_for_target(target_file)
                    .map(|name| format!("{name} "))
                    .unwrap_or_default();
                Ok(Some(format!(
                    "{}{}:{}% W:{}% {}",
                    prefix,
                    entry.non_weekly_label,
                    entry.non_weekly_remaining,
                    entry.weekly_remaining,
                    weekly_reset_iso
                )))
            }
            Err(err) => {
                if debug_mode {
                    eprintln!("{err}");
                }
                Ok(None)
            }
        };
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);

    let usage_request = UsageRequest {
        target_file: target_file.to_path_buf(),
        refresh_on_401: !no_refresh_auth,
        base_url,
        connect_timeout_seconds: connect_timeout,
        max_time_seconds: max_time,
    };

    let usage = match fetch_usage(&usage_request) {
        Ok(value) => value,
        Err(err) => {
            if debug_mode {
                eprintln!("{err}");
            }
            return Ok(None);
        }
    };

    let _ = writeback::write_weekly(target_file, &usage.json);
    if is_auth_file(target_file) {
        let _ = auth::sync::run();
    }

    let usage_data = match render::parse_usage(&usage.json) {
        Some(value) => value,
        None => return Ok(None),
    };
    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);
    let prefix = cache::secret_name_for_target(target_file)
        .map(|name| format!("{name} "))
        .unwrap_or_default();
    let weekly_reset_iso = render::format_epoch_local_datetime(weekly.weekly_reset_epoch)
        .unwrap_or_else(|| "?".to_string());

    Ok(Some(format!(
        "{}{}:{}% W:{}% {}",
        prefix, weekly.non_weekly_label, weekly.non_weekly_remaining, weekly.weekly_remaining, weekly_reset_iso
    )))
}

fn resolve_target(secret: Option<&str>) -> std::result::Result<PathBuf, i32> {
    if let Some(secret_name) = secret {
        if secret_name.is_empty() || secret_name.contains('/') || secret_name.contains("..") {
            eprintln!("codex-rate-limits: invalid secret file name: {secret_name}");
            return Err(64);
        }
        let secret_dir = crate::paths::resolve_secret_dir().unwrap_or_default();
        return Ok(secret_dir.join(secret_name));
    }

    if let Some(auth_file) = crate::paths::resolve_auth_file() {
        return Ok(auth_file);
    }

    Err(1)
}

fn is_auth_file(target_file: &Path) -> bool {
    if let Some(auth_file) = crate::paths::resolve_auth_file() {
        return auth_file == target_file;
    }
    false
}

fn env_timeout(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default)
}

struct Row {
    name: String,
    window_label: String,
    non_weekly_remaining: i64,
    non_weekly_reset_epoch: Option<i64>,
    weekly_remaining: i64,
    weekly_reset_epoch: Option<i64>,
    weekly_reset_iso: String,
}

impl Row {
    fn empty(name: String) -> Self {
        Self {
            name,
            window_label: String::new(),
            non_weekly_remaining: -1,
            non_weekly_reset_epoch: None,
            weekly_remaining: -1,
            weekly_reset_epoch: None,
            weekly_reset_iso: String::new(),
        }
    }

    fn sort_key(&self) -> (i32, i64, String) {
        if let Some(epoch) = self.weekly_reset_epoch {
            (0, epoch, self.name.clone())
        } else {
            (1, i64::MAX, self.name.clone())
        }
    }
}

struct ParsedOneLine {
    window_label: String,
    non_weekly_remaining: i64,
    weekly_remaining: i64,
    weekly_reset_iso: String,
}

fn parse_one_line_output(line: &str) -> Option<ParsedOneLine> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }

    let window_field = parts[parts.len() - 3];
    let weekly_field = parts[parts.len() - 2];
    let reset_iso = parts[parts.len() - 1].to_string();

    let window_label = window_field.split(':').next()?.trim_matches('"').to_string();
    let non_weekly_remaining = window_field.split(':').nth(1)?;
    let non_weekly_remaining = non_weekly_remaining.trim_end_matches('%').parse::<i64>().ok()?;

    let weekly_remaining = weekly_field.trim_start_matches("W:").trim_end_matches('%');
    let weekly_remaining = weekly_remaining.parse::<i64>().ok()?;

    Some(ParsedOneLine {
        window_label,
        non_weekly_remaining,
        weekly_remaining,
        weekly_reset_iso: reset_iso,
    })
}
