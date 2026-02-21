use std::fs;
use std::path::{Path, PathBuf};

use crate::auth;
use crate::fs as gemini_fs;
use crate::paths;
use crate::rate_limits::client::{UsageRequest, fetch_usage};

pub mod ansi;
pub mod client;
pub mod render;

#[derive(Clone, Debug, Default)]
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

#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub non_weekly_label: String,
    pub non_weekly_remaining: i64,
    pub non_weekly_reset_epoch: Option<i64>,
    pub weekly_remaining: i64,
    pub weekly_reset_epoch: i64,
}

pub const DIAG_SCHEMA_VERSION: &str = "gemini-cli.diag.rate-limits.v1";
pub const DIAG_COMMAND: &str = "diag rate-limits";

#[derive(Clone, Debug)]
struct RateLimitSummary {
    non_weekly_label: String,
    non_weekly_remaining: i64,
    non_weekly_reset_epoch: Option<i64>,
    weekly_remaining: i64,
    weekly_reset_epoch: i64,
}

#[derive(Clone, Debug)]
struct JsonResultItem {
    name: String,
    target_file: String,
    status: String,
    ok: bool,
    source: String,
    summary: Option<RateLimitSummary>,
    raw_usage: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

struct Row {
    name: String,
    window_label: String,
    non_weekly_remaining: i64,
    non_weekly_reset_epoch: Option<i64>,
    weekly_remaining: i64,
    weekly_reset_epoch: Option<i64>,
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

pub fn run(args: &RateLimitsOptions) -> i32 {
    let cached_mode = args.cached;
    let output_json = args.json;

    if args.async_mode {
        if !cached_mode {
            maybe_sync_all_mode_auth_silent(args.debug);
        }
        if output_json {
            return run_async_json_mode(args);
        }
        return run_async_mode(args);
    }

    if cached_mode {
        if output_json {
            emit_error_json(
                "invalid-flag-combination",
                "gemini-rate-limits: --json is not supported with --cached",
                Some(json_obj(vec![(
                    "flags".to_string(),
                    json_array(vec![json_string("--json"), json_string("--cached")]),
                )])),
            );
            return 64;
        }
        if args.clear_cache {
            eprintln!("gemini-rate-limits: -c is not compatible with --cached");
            return 64;
        }
    }

    if output_json && args.one_line {
        emit_error_json(
            "invalid-flag-combination",
            "gemini-rate-limits: --one-line is not compatible with --json",
            Some(json_obj(vec![(
                "flags".to_string(),
                json_array(vec![json_string("--one-line"), json_string("--json")]),
            )])),
        );
        return 64;
    }

    if args.clear_cache
        && let Err(err) = clear_starship_cache()
    {
        if output_json {
            emit_error_json("cache-clear-failed", &err, None);
        } else {
            eprintln!("{err}");
        }
        return 1;
    }

    let default_all_enabled = env_truthy("GEMINI_RATE_LIMITS_DEFAULT_ALL_ENABLED");
    let all_mode = args.all
        || (!args.cached
            && !output_json
            && args.secret.is_none()
            && default_all_enabled
            && !args.async_mode);

    if all_mode {
        if !cached_mode {
            maybe_sync_all_mode_auth_silent(args.debug);
        }
        if args.secret.is_some() {
            eprintln!(
                "gemini-rate-limits: usage: gemini-rate-limits [-c] [--cached] [--no-refresh-auth] [--json] [--one-line] [--all] [secret.json]"
            );
            return 64;
        }
        if output_json {
            return run_all_json_mode(args, cached_mode);
        }
        return run_all_mode(args, cached_mode);
    }

    run_single_mode(args, cached_mode, output_json)
}

fn maybe_sync_all_mode_auth_silent(debug_mode: bool) {
    if let Err(err) = sync_auth_silent()
        && debug_mode
    {
        eprintln!("{err}");
    }
}

fn sync_auth_silent() -> Result<(), String> {
    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok(()),
    };

    if !auth_file.is_file() {
        return Ok(());
    }

    let auth_key = match auth::identity_key_from_auth_file(&auth_file) {
        Ok(Some(key)) => key,
        _ => return Ok(()),
    };

    let auth_last_refresh = auth::last_refresh_from_auth_file(&auth_file).ok().flatten();
    let auth_contents = fs::read(&auth_file)
        .map_err(|_| format!("gemini-rate-limits: failed to read {}", auth_file.display()))?;

    if let Some(secret_dir) = paths::resolve_secret_dir()
        && let Ok(entries) = fs::read_dir(&secret_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let candidate_key = match auth::identity_key_from_auth_file(&path) {
                Ok(Some(key)) => key,
                _ => continue,
            };
            if candidate_key != auth_key {
                continue;
            }

            let secret_contents = fs::read(&path)
                .map_err(|_| format!("gemini-rate-limits: failed to read {}", path.display()))?;
            if secret_contents == auth_contents {
                continue;
            }

            auth::write_atomic(&path, &auth_contents, auth::SECRET_FILE_MODE)
                .map_err(|_| format!("gemini-rate-limits: failed to write {}", path.display()))?;

            if let Some(timestamp_path) = sync_timestamp_path(&path) {
                let _ = auth::write_timestamp(&timestamp_path, auth_last_refresh.as_deref());
            }
        }
    }

    if let Some(auth_timestamp) = sync_timestamp_path(&auth_file) {
        let _ = auth::write_timestamp(&auth_timestamp, auth_last_refresh.as_deref());
    }

    Ok(())
}

fn sync_timestamp_path(target_file: &Path) -> Option<PathBuf> {
    let cache_dir = paths::resolve_secret_cache_dir()?;
    let name = target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json");
    Some(cache_dir.join(format!("{name}.timestamp")))
}

fn run_single_mode(args: &RateLimitsOptions, cached_mode: bool, output_json: bool) -> i32 {
    let target_file = match resolve_single_target(args.secret.as_deref()) {
        Ok(path) => path,
        Err(message) => {
            if output_json {
                emit_error_json("target-not-found", &message, None);
            } else {
                eprintln!("{message}");
            }
            return 1;
        }
    };

    if cached_mode {
        let cache_entry = match read_cache_entry(&target_file) {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("{err}");
                return 1;
            }
        };
        let name = secret_name_for_target(&target_file).unwrap_or_else(|| {
            target_file
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("auth")
                .to_string()
        });
        let summary = RateLimitSummary {
            non_weekly_label: cache_entry.non_weekly_label,
            non_weekly_remaining: cache_entry.non_weekly_remaining,
            non_weekly_reset_epoch: cache_entry.non_weekly_reset_epoch,
            weekly_remaining: cache_entry.weekly_remaining,
            weekly_reset_epoch: cache_entry.weekly_reset_epoch,
        };
        if args.one_line {
            let line = render_line_for_summary(&name, &summary, true, "%m-%d %H:%M");
            println!("{line}");
        } else {
            print_rate_limits_remaining(&summary, "%m-%d %H:%M");
        }
        return 0;
    }

    match collect_summary_from_network(&target_file, !args.no_refresh_auth) {
        Ok((summary, raw_usage)) => {
            if output_json {
                let item = JsonResultItem {
                    name: secret_name_for_target(&target_file)
                        .unwrap_or_else(|| "auth".to_string()),
                    target_file: target_file_name(&target_file),
                    status: "ok".to_string(),
                    ok: true,
                    source: "network".to_string(),
                    summary: Some(summary.clone()),
                    raw_usage,
                    error_code: None,
                    error_message: None,
                };
                emit_single_envelope("single", true, &item);
            } else {
                let name = secret_name_for_target(&target_file).unwrap_or_else(|| {
                    target_file
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("auth")
                        .to_string()
                });
                if args.one_line {
                    let line =
                        render_line_for_summary(&name, &summary, args.one_line, "%m-%d %H:%M");
                    println!("{line}");
                } else {
                    print_rate_limits_remaining(&summary, "%m-%d %H:%M");
                }
            }
            0
        }
        Err(err) => {
            if output_json {
                emit_error_json(&err.code, &err.message, err.details);
            } else {
                eprintln!("{}", err.message);
            }
            err.exit_code
        }
    }
}

fn run_all_mode(args: &RateLimitsOptions, cached_mode: bool) -> i32 {
    let secret_files = match collect_secret_files() {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let current_name = current_secret_basename(&secret_files);

    let mut rc = 0;
    let mut rows: Vec<Row> = Vec::new();
    let mut window_labels = std::collections::HashSet::new();

    for target in secret_files {
        let name = secret_name_for_target(&target).unwrap_or_else(|| target_file_name(&target));
        let mut row = Row::empty(name.clone());

        if cached_mode {
            match read_cache_entry(&target) {
                Ok(summary) => {
                    row.window_label = summary.non_weekly_label.clone();
                    row.non_weekly_remaining = summary.non_weekly_remaining;
                    row.non_weekly_reset_epoch = summary.non_weekly_reset_epoch;
                    row.weekly_remaining = summary.weekly_remaining;
                    row.weekly_reset_epoch = Some(summary.weekly_reset_epoch);
                    window_labels.insert(row.window_label.clone());
                }
                Err(err) => {
                    eprintln!("{name}: {err}");
                    rc = 1;
                }
            }
            rows.push(row);
            continue;
        }

        match collect_summary_from_network(&target, !args.no_refresh_auth) {
            Ok((summary, _raw)) => {
                row.window_label = summary.non_weekly_label.clone();
                row.non_weekly_remaining = summary.non_weekly_remaining;
                row.non_weekly_reset_epoch = summary.non_weekly_reset_epoch;
                row.weekly_remaining = summary.weekly_remaining;
                row.weekly_reset_epoch = Some(summary.weekly_reset_epoch);
                window_labels.insert(row.window_label.clone());
            }
            Err(err) => {
                eprintln!("{name}: {}", err.message);
                rc = 1;
            }
        }
        rows.push(row);
    }

    println!("\n🚦 Gemini rate limits for all accounts\n");

    let mut non_weekly_header = "Non-weekly".to_string();
    let multiple_labels = window_labels.len() != 1;
    if !multiple_labels && let Some(label) = window_labels.iter().next() {
        non_weekly_header = label.clone();
    }

    let now_epoch = now_epoch_seconds();

    println!(
        "{:<15}  {:>8}  {:>7}  {:>8}  {:>7}  {:<18}",
        "Name", non_weekly_header, "Left", "Weekly", "Left", "Reset"
    );
    println!("----------------------------------------------------------------------------");

    rows.sort_by_key(|row| row.sort_key());

    for row in rows {
        let display_non_weekly = if multiple_labels && !row.window_label.is_empty() {
            if row.non_weekly_remaining >= 0 {
                format!("{}:{}%", row.window_label, row.non_weekly_remaining)
            } else {
                "-".to_string()
            }
        } else if row.non_weekly_remaining >= 0 {
            format!("{}%", row.non_weekly_remaining)
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
            .and_then(render::format_epoch_local_datetime_with_offset)
            .unwrap_or_else(|| "-".to_string());

        let non_weekly_display = ansi::format_percent_cell(&display_non_weekly, 8, None);
        let weekly_display = if row.weekly_remaining >= 0 {
            ansi::format_percent_cell(&format!("{}%", row.weekly_remaining), 8, None)
        } else {
            ansi::format_percent_cell("-", 8, None)
        };

        let is_current = current_name.as_deref() == Some(row.name.as_str());
        let name_display = ansi::format_name_cell(&row.name, 15, is_current, None);

        println!(
            "{}  {}  {:>7}  {}  {:>7}  {:<18}",
            name_display,
            non_weekly_display,
            non_weekly_left,
            weekly_display,
            weekly_left,
            reset_display
        );
    }

    rc
}

fn run_all_json_mode(args: &RateLimitsOptions, cached_mode: bool) -> i32 {
    let secret_files = match collect_secret_files() {
        Ok(value) => value,
        Err(err) => {
            emit_error_json("secret-discovery-failed", &err, None);
            return 1;
        }
    };

    let mut items: Vec<JsonResultItem> = Vec::new();
    let mut rc = 0;

    for target in secret_files {
        let name = secret_name_for_target(&target).unwrap_or_else(|| target_file_name(&target));
        if cached_mode {
            match read_cache_entry(&target) {
                Ok(entry) => items.push(JsonResultItem {
                    name,
                    target_file: target_file_name(&target),
                    status: "ok".to_string(),
                    ok: true,
                    source: "cache".to_string(),
                    summary: Some(RateLimitSummary {
                        non_weekly_label: entry.non_weekly_label,
                        non_weekly_remaining: entry.non_weekly_remaining,
                        non_weekly_reset_epoch: entry.non_weekly_reset_epoch,
                        weekly_remaining: entry.weekly_remaining,
                        weekly_reset_epoch: entry.weekly_reset_epoch,
                    }),
                    raw_usage: None,
                    error_code: None,
                    error_message: None,
                }),
                Err(err) => {
                    rc = 1;
                    items.push(JsonResultItem {
                        name,
                        target_file: target_file_name(&target),
                        status: "error".to_string(),
                        ok: false,
                        source: "cache".to_string(),
                        summary: None,
                        raw_usage: None,
                        error_code: Some("cache-read-failed".to_string()),
                        error_message: Some(err),
                    });
                }
            }
            continue;
        }

        match collect_summary_from_network(&target, !args.no_refresh_auth) {
            Ok((summary, raw_usage)) => items.push(JsonResultItem {
                name,
                target_file: target_file_name(&target),
                status: "ok".to_string(),
                ok: true,
                source: "network".to_string(),
                summary: Some(summary),
                raw_usage,
                error_code: None,
                error_message: None,
            }),
            Err(err) => {
                rc = 1;
                items.push(JsonResultItem {
                    name,
                    target_file: target_file_name(&target),
                    status: "error".to_string(),
                    ok: false,
                    source: "network".to_string(),
                    summary: None,
                    raw_usage: None,
                    error_code: Some(err.code),
                    error_message: Some(err.message),
                });
            }
        }
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    emit_collection_envelope("all", rc == 0, &items);
    rc
}

fn run_async_mode(args: &RateLimitsOptions) -> i32 {
    if args.one_line {
        eprintln!("gemini-rate-limits: --async does not support --one-line");
        return 64;
    }
    if let Some(secret) = args.secret.as_deref() {
        eprintln!("gemini-rate-limits: --async does not accept positional args: {secret}");
        eprintln!("hint: async always queries all secrets under GEMINI_SECRET_DIR");
        return 64;
    }
    if args.clear_cache && args.cached {
        eprintln!("gemini-rate-limits: --async: -c is not compatible with --cached");
        return 64;
    }
    if args.clear_cache
        && let Err(err) = clear_starship_cache()
    {
        eprintln!("{err}");
        return 1;
    }
    run_all_mode(args, args.cached)
}

fn run_async_json_mode(args: &RateLimitsOptions) -> i32 {
    if args.one_line {
        emit_error_json(
            "invalid-flag-combination",
            "gemini-rate-limits: --async does not support --one-line",
            Some(json_obj(vec![
                ("flag".to_string(), json_string("--one-line")),
                ("mode".to_string(), json_string("async")),
            ])),
        );
        return 64;
    }
    if let Some(secret) = args.secret.as_deref() {
        emit_error_json(
            "invalid-positional-arg",
            &format!("gemini-rate-limits: --async does not accept positional args: {secret}"),
            Some(json_obj(vec![
                ("secret".to_string(), json_string(secret)),
                ("mode".to_string(), json_string("async")),
            ])),
        );
        return 64;
    }
    if args.clear_cache && args.cached {
        emit_error_json(
            "invalid-flag-combination",
            "gemini-rate-limits: --async: -c is not compatible with --cached",
            Some(json_obj(vec![(
                "flags".to_string(),
                json_array(vec![
                    json_string("--async"),
                    json_string("--cached"),
                    json_string("-c"),
                ]),
            )])),
        );
        return 64;
    }
    if args.clear_cache
        && let Err(err) = clear_starship_cache()
    {
        emit_error_json("cache-clear-failed", &err, None);
        return 1;
    }

    let secret_files = match collect_secret_files() {
        Ok(value) => value,
        Err(err) => {
            emit_error_json("secret-discovery-failed", &err, None);
            return 1;
        }
    };

    let mut items: Vec<JsonResultItem> = Vec::new();
    let mut rc = 0;
    for target in secret_files {
        let name = secret_name_for_target(&target).unwrap_or_else(|| target_file_name(&target));
        match collect_summary_from_network(&target, !args.no_refresh_auth) {
            Ok((summary, raw_usage)) => items.push(JsonResultItem {
                name,
                target_file: target_file_name(&target),
                status: "ok".to_string(),
                ok: true,
                source: "network".to_string(),
                summary: Some(summary),
                raw_usage,
                error_code: None,
                error_message: None,
            }),
            Err(err) => {
                if err.code == "missing-access-token"
                    && let Ok(cached) = read_cache_entry(&target)
                {
                    items.push(JsonResultItem {
                        name,
                        target_file: target_file_name(&target),
                        status: "ok".to_string(),
                        ok: true,
                        source: "cache-fallback".to_string(),
                        summary: Some(RateLimitSummary {
                            non_weekly_label: cached.non_weekly_label,
                            non_weekly_remaining: cached.non_weekly_remaining,
                            non_weekly_reset_epoch: cached.non_weekly_reset_epoch,
                            weekly_remaining: cached.weekly_remaining,
                            weekly_reset_epoch: cached.weekly_reset_epoch,
                        }),
                        raw_usage: None,
                        error_code: None,
                        error_message: None,
                    });
                    continue;
                }

                rc = 1;
                items.push(JsonResultItem {
                    name,
                    target_file: target_file_name(&target),
                    status: "error".to_string(),
                    ok: false,
                    source: "network".to_string(),
                    summary: None,
                    raw_usage: None,
                    error_code: Some(err.code),
                    error_message: Some(err.message),
                });
            }
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    emit_collection_envelope("async", rc == 0, &items);
    rc
}

pub fn clear_starship_cache() -> Result<(), String> {
    let root =
        cache_root().ok_or_else(|| "gemini-rate-limits: cache root unavailable".to_string())?;
    if !root.is_absolute() {
        return Err(format!(
            "gemini-rate-limits: refusing to clear cache with non-absolute cache root: {}",
            root.display()
        ));
    }
    if root == Path::new("/") {
        return Err(format!(
            "gemini-rate-limits: refusing to clear cache with invalid cache root: {}",
            root.display()
        ));
    }

    let cache_dir = root.join("gemini").join("starship-rate-limits");
    let cache_dir_str = cache_dir.to_string_lossy();
    if !cache_dir_str.ends_with("/gemini/starship-rate-limits") {
        return Err(format!(
            "gemini-rate-limits: refusing to clear unexpected cache dir: {}",
            cache_dir.display()
        ));
    }

    if cache_dir.is_dir() {
        let _ = fs::remove_dir_all(&cache_dir);
    }
    Ok(())
}

pub fn cache_file_for_target(target_file: &Path) -> Result<PathBuf, String> {
    let cache_dir = starship_cache_dir()
        .ok_or_else(|| "gemini-rate-limits: cache dir unavailable".to_string())?;

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

    let hash = gemini_fs::sha256_file(target_file).map_err(|err| err.to_string())?;
    Ok(cache_dir.join(format!("auth_{}.kv", hash.to_lowercase())))
}

pub fn secret_name_for_target(target_file: &Path) -> Option<String> {
    let secret_dir = paths::resolve_secret_dir()?;
    if target_file.starts_with(&secret_dir) {
        return secret_file_basename(target_file).ok();
    }
    secret_name_for_auth(target_file, &secret_dir)
}

pub fn read_cache_entry(target_file: &Path) -> Result<CacheEntry, String> {
    let cache_file = cache_file_for_target(target_file)?;
    if !cache_file.is_file() {
        return Err(format!(
            "gemini-rate-limits: cache not found (run gemini-rate-limits without --cached, or gemini-cli starship, to populate): {}",
            cache_file.display()
        ));
    }

    let content = fs::read_to_string(&cache_file).map_err(|_| {
        format!(
            "gemini-rate-limits: failed to read cache: {}",
            cache_file.display()
        )
    })?;
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
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            return Err(format!(
                "gemini-rate-limits: invalid cache (missing non-weekly data): {}",
                cache_file.display()
            ));
        }
    };
    let non_weekly_remaining = match non_weekly_remaining {
        Some(value) => value,
        None => {
            return Err(format!(
                "gemini-rate-limits: invalid cache (missing non-weekly data): {}",
                cache_file.display()
            ));
        }
    };
    let weekly_remaining = match weekly_remaining {
        Some(value) => value,
        None => {
            return Err(format!(
                "gemini-rate-limits: invalid cache (missing weekly data): {}",
                cache_file.display()
            ));
        }
    };
    let weekly_reset_epoch = match weekly_reset_epoch {
        Some(value) => value,
        None => {
            return Err(format!(
                "gemini-rate-limits: invalid cache (missing weekly data): {}",
                cache_file.display()
            ));
        }
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
) -> Result<(), String> {
    let cache_file = cache_file_for_target(target_file)?;
    if let Some(parent) = cache_file.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
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
    gemini_fs::write_atomic(&cache_file, data.as_bytes(), gemini_fs::SECRET_FILE_MODE)
        .map_err(|err| err.to_string())
}

const DEFAULT_CODE_ASSIST_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
const DEFAULT_CODE_ASSIST_API_VERSION: &str = "v1internal";
const DEFAULT_CODE_ASSIST_PROJECT: &str = "projects/default";

fn run_code_assist_endpoint() -> String {
    env_non_empty("CODE_ASSIST_ENDPOINT")
        .or_else(|| env_non_empty("GEMINI_CODE_ASSIST_ENDPOINT"))
        .unwrap_or_else(|| DEFAULT_CODE_ASSIST_ENDPOINT.to_string())
}

fn run_code_assist_api_version() -> String {
    env_non_empty("CODE_ASSIST_API_VERSION")
        .or_else(|| env_non_empty("GEMINI_CODE_ASSIST_API_VERSION"))
        .unwrap_or_else(|| DEFAULT_CODE_ASSIST_API_VERSION.to_string())
}

fn run_code_assist_project() -> String {
    let raw = env_non_empty("GEMINI_CODE_ASSIST_PROJECT")
        .or_else(|| env_non_empty("GOOGLE_CLOUD_PROJECT"))
        .or_else(|| env_non_empty("GOOGLE_CLOUD_PROJECT_ID"))
        .unwrap_or_else(|| DEFAULT_CODE_ASSIST_PROJECT.to_string());

    if raw.starts_with("projects/") {
        raw
    } else {
        format!("projects/{raw}")
    }
}

fn run_connect_timeout() -> u64 {
    std::env::var("GEMINI_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(2)
}

fn run_max_time() -> u64 {
    std::env::var("GEMINI_RATE_LIMITS_CURL_MAX_TIME_SECONDS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(8)
}

fn collect_summary_from_network(
    target_file: &Path,
    refresh_on_401: bool,
) -> Result<(RateLimitSummary, Option<String>), RunError> {
    let request = UsageRequest {
        target_file: target_file.to_path_buf(),
        refresh_on_401,
        endpoint: run_code_assist_endpoint(),
        api_version: run_code_assist_api_version(),
        project: run_code_assist_project(),
        connect_timeout_seconds: run_connect_timeout(),
        max_time_seconds: run_max_time(),
    };
    let usage = fetch_usage(&request).map_err(|message| {
        let (code, exit_code) = if message.contains("missing access_token") {
            ("missing-access-token".to_string(), 2)
        } else {
            ("request-failed".to_string(), 3)
        };
        RunError {
            code,
            message,
            details: None,
            exit_code,
        }
    })?;

    let usage_data = render::parse_usage(&usage.body).ok_or_else(|| RunError {
        code: "invalid-usage-payload".to_string(),
        message: "gemini-rate-limits: invalid usage payload".to_string(),
        details: Some(json_obj(vec![(
            "raw_usage".to_string(),
            usage.body.clone(),
        )])),
        exit_code: 3,
    })?;
    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);
    let summary = RateLimitSummary {
        non_weekly_label: weekly.non_weekly_label.clone(),
        non_weekly_remaining: weekly.non_weekly_remaining,
        non_weekly_reset_epoch: weekly.non_weekly_reset_epoch,
        weekly_remaining: weekly.weekly_remaining,
        weekly_reset_epoch: weekly.weekly_reset_epoch,
    };

    let now_epoch = now_epoch_seconds();
    if now_epoch > 0 {
        let _ = write_starship_cache(
            target_file,
            now_epoch,
            &summary.non_weekly_label,
            summary.non_weekly_remaining,
            summary.weekly_remaining,
            summary.weekly_reset_epoch,
            summary.non_weekly_reset_epoch,
        );
    }

    let raw_usage = if usage.body.trim_start().starts_with('{') {
        Some(usage.body)
    } else {
        None
    };

    Ok((summary, raw_usage))
}

fn collect_secret_files() -> Result<Vec<PathBuf>, String> {
    let secret_dir = paths::resolve_secret_dir().unwrap_or_default();
    if !secret_dir.is_dir() {
        return Err(format!(
            "gemini-rate-limits: GEMINI_SECRET_DIR not found: {}",
            secret_dir.display()
        ));
    }

    let mut files: Vec<PathBuf> = fs::read_dir(&secret_dir)
        .map_err(|err| format!("gemini-rate-limits: failed to read GEMINI_SECRET_DIR: {err}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect();

    files.sort();

    if files.is_empty() {
        return Err(format!(
            "gemini-rate-limits: no secrets found under GEMINI_SECRET_DIR: {}",
            secret_dir.display()
        ));
    }

    Ok(files)
}

fn resolve_single_target(secret: Option<&str>) -> Result<PathBuf, String> {
    if let Some(raw) = secret {
        if raw.trim().is_empty() {
            return Err("gemini-rate-limits: empty secret target".to_string());
        }
        let path = if raw.contains('/') || raw.starts_with('.') {
            PathBuf::from(raw)
        } else if let Some(secret_dir) = paths::resolve_secret_dir() {
            let mut file = raw.to_string();
            if !file.ends_with(".json") {
                file.push_str(".json");
            }
            secret_dir.join(file)
        } else {
            PathBuf::from(raw)
        };

        if !path.is_file() {
            return Err(format!(
                "gemini-rate-limits: target file not found: {}",
                path.display()
            ));
        }
        return Ok(path);
    }

    let auth = paths::resolve_auth_file().ok_or_else(|| {
        "gemini-rate-limits: GEMINI_AUTH_FILE is not configured and no secret provided".to_string()
    })?;
    if !auth.is_file() {
        return Err(format!(
            "gemini-rate-limits: target file not found: {}",
            auth.display()
        ));
    }
    Ok(auth)
}

fn secret_file_basename(path: &Path) -> Result<String, String> {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let base = file.trim_end_matches(".json");
    if base.is_empty() {
        return Err("missing secret basename".to_string());
    }
    Ok(base.to_string())
}

fn cache_key(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("missing cache key name".to_string());
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
        return Err("invalid cache key name".to_string());
    }
    Ok(key)
}

fn secret_name_for_auth(auth_file: &Path, secret_dir: &Path) -> Option<String> {
    let auth_key = auth::identity_key_from_auth_file(auth_file)
        .ok()
        .flatten()?;
    let entries = fs::read_dir(secret_dir).ok()?;
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

fn current_secret_basename(secret_files: &[PathBuf]) -> Option<String> {
    let auth_file = paths::resolve_auth_file()?;
    if !auth_file.is_file() {
        return None;
    }

    let auth_hash = gemini_fs::sha256_file(&auth_file).ok();
    if let Some(auth_hash) = auth_hash.as_deref() {
        for secret_file in secret_files {
            if let Ok(secret_hash) = gemini_fs::sha256_file(secret_file)
                && secret_hash == auth_hash
                && let Ok(name) = secret_file_basename(secret_file)
            {
                return Some(name);
            }
        }
    }

    let auth_key = auth::identity_key_from_auth_file(&auth_file).ok().flatten();
    if let Some(auth_key) = auth_key.as_deref() {
        for secret_file in secret_files {
            if let Ok(Some(candidate_key)) = auth::identity_key_from_auth_file(secret_file)
                && candidate_key == auth_key
                && let Ok(name) = secret_file_basename(secret_file)
            {
                return Some(name);
            }
        }
    }

    None
}

fn starship_cache_dir() -> Option<PathBuf> {
    let root = cache_root()?;
    Some(root.join("gemini").join("starship-rate-limits"))
}

fn cache_root() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ZSH_CACHE_DIR")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    let zdotdir = paths::resolve_zdotdir()?;
    Some(zdotdir.join("cache"))
}

fn render_line_for_summary(
    name: &str,
    summary: &RateLimitSummary,
    one_line: bool,
    time_format: &str,
) -> String {
    let reset = render::format_epoch_local(summary.weekly_reset_epoch, time_format)
        .unwrap_or_else(|| "?".to_string());
    let token_5h = format!(
        "{}:{}%",
        summary.non_weekly_label, summary.non_weekly_remaining
    );
    let token_weekly = format!("W:{}%", summary.weekly_remaining);

    if one_line {
        return format!("{name} {token_5h} {token_weekly} {reset}");
    }
    format!("{token_5h} {token_weekly} {reset}")
}

fn print_rate_limits_remaining(summary: &RateLimitSummary, time_format: &str) {
    println!("Rate limits remaining");
    let non_weekly_reset = summary
        .non_weekly_reset_epoch
        .and_then(|epoch| render::format_epoch_local(epoch, time_format))
        .unwrap_or_else(|| "?".to_string());
    let weekly_reset = render::format_epoch_local(summary.weekly_reset_epoch, time_format)
        .unwrap_or_else(|| "?".to_string());
    println!(
        "{} {}% • {}",
        summary.non_weekly_label, summary.non_weekly_remaining, non_weekly_reset
    );
    println!("Weekly {}% • {}", summary.weekly_remaining, weekly_reset);
}

fn target_file_name(path: &Path) -> String {
    if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
        name.to_string()
    } else {
        path.to_string_lossy().to_string()
    }
}

fn now_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn env_truthy(key: &str) -> bool {
    match std::env::var(key) {
        Ok(raw) => {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        }
        Err(_) => false,
    }
}

struct RunError {
    code: String,
    message: String,
    details: Option<String>,
    exit_code: i32,
}

fn emit_single_envelope(mode: &str, ok: bool, result: &JsonResultItem) {
    println!(
        "{{\"schema_version\":\"{}\",\"command\":\"{}\",\"mode\":\"{}\",\"ok\":{},\"result\":{}}}",
        DIAG_SCHEMA_VERSION,
        DIAG_COMMAND,
        json_escape(mode),
        if ok { "true" } else { "false" },
        result.to_json()
    );
}

fn emit_collection_envelope(mode: &str, ok: bool, results: &[JsonResultItem]) {
    let mut body = String::new();
    body.push('[');
    for (index, result) in results.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&result.to_json());
    }
    body.push(']');

    println!(
        "{{\"schema_version\":\"{}\",\"command\":\"{}\",\"mode\":\"{}\",\"ok\":{},\"results\":{}}}",
        DIAG_SCHEMA_VERSION,
        DIAG_COMMAND,
        json_escape(mode),
        if ok { "true" } else { "false" },
        body
    );
}

fn emit_error_json(code: &str, message: &str, details: Option<String>) {
    print!(
        "{{\"schema_version\":\"{}\",\"command\":\"{}\",\"ok\":false,\"error\":{{\"code\":\"{}\",\"message\":\"{}\"",
        DIAG_SCHEMA_VERSION,
        DIAG_COMMAND,
        json_escape(code),
        json_escape(message),
    );
    if let Some(details) = details {
        print!(",\"details\":{}", details);
    }
    println!("}}}}");
}

impl JsonResultItem {
    fn to_json(&self) -> String {
        let mut s = String::new();
        s.push('{');
        push_field(&mut s, "name", &json_string(&self.name), true);
        push_field(
            &mut s,
            "target_file",
            &json_string(&self.target_file),
            false,
        );
        push_field(&mut s, "status", &json_string(&self.status), false);
        push_field(&mut s, "ok", if self.ok { "true" } else { "false" }, false);
        push_field(&mut s, "source", &json_string(&self.source), false);

        if let Some(summary) = &self.summary {
            push_field(&mut s, "summary", &summary.to_json(), false);
        }

        if let Some(raw_usage) = &self.raw_usage {
            let trimmed = raw_usage.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                push_field(&mut s, "raw_usage", trimmed, false);
            } else {
                push_field(&mut s, "raw_usage", &json_string(trimmed), false);
            }
        } else {
            push_field(&mut s, "raw_usage", "null", false);
        }

        if let (Some(code), Some(message)) = (&self.error_code, &self.error_message) {
            let error_json = format!(
                "{{\"code\":\"{}\",\"message\":\"{}\"}}",
                json_escape(code),
                json_escape(message)
            );
            push_field(&mut s, "error", &error_json, false);
        }

        s.push('}');
        s
    }
}

impl RateLimitSummary {
    fn to_json(&self) -> String {
        let mut s = String::new();
        s.push('{');
        push_field(
            &mut s,
            "non_weekly_label",
            &json_string(&self.non_weekly_label),
            true,
        );
        push_field(
            &mut s,
            "non_weekly_remaining",
            &self.non_weekly_remaining.to_string(),
            false,
        );
        match self.non_weekly_reset_epoch {
            Some(value) => push_field(&mut s, "non_weekly_reset_epoch", &value.to_string(), false),
            None => push_field(&mut s, "non_weekly_reset_epoch", "null", false),
        }
        push_field(
            &mut s,
            "weekly_remaining",
            &self.weekly_remaining.to_string(),
            false,
        );
        push_field(
            &mut s,
            "weekly_reset_epoch",
            &self.weekly_reset_epoch.to_string(),
            false,
        );
        s.push('}');
        s
    }
}

fn push_field(buf: &mut String, key: &str, value_json: &str, first: bool) {
    if !first {
        buf.push(',');
    }
    buf.push('"');
    buf.push_str(&json_escape(key));
    buf.push_str("\":");
    buf.push_str(value_json);
}

fn json_string(raw: &str) -> String {
    format!("\"{}\"", json_escape(raw))
}

fn json_array(values: Vec<String>) -> String {
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(value);
    }
    out.push(']');
    out
}

fn json_obj(fields: Vec<(String, String)>) -> String {
    let mut out = String::from("{");
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(key));
        out.push_str("\":");
        out.push_str(value);
    }
    out.push('}');
    out
}

fn json_escape(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct EnvGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: test-scoped env mutation.
            unsafe { std::env::set_var(key, value) };
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.old.take() {
                // SAFETY: test-scoped env restore.
                unsafe { std::env::set_var(self.key, value) };
            } else {
                // SAFETY: test-scoped env restore.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    #[test]
    fn cache_key_normalizes_and_rejects_empty() {
        assert_eq!(cache_key("Alpha.Work").expect("key"), "alpha_work");
        assert!(cache_key("___").is_err());
    }

    #[test]
    fn secret_file_basename_requires_non_empty_name() {
        assert_eq!(
            secret_file_basename(Path::new("/tmp/alpha.json")).expect("basename"),
            "alpha"
        );
        assert!(secret_file_basename(Path::new("/tmp/.json")).is_err());
    }

    #[test]
    fn env_truthy_accepts_expected_variants() {
        let _v1 = EnvGuard::set("GEMINI_TEST_TRUTHY", "true");
        assert!(env_truthy("GEMINI_TEST_TRUTHY"));
        let _v2 = EnvGuard::set("GEMINI_TEST_TRUTHY", "ON");
        assert!(env_truthy("GEMINI_TEST_TRUTHY"));
        let _v3 = EnvGuard::set("GEMINI_TEST_TRUTHY", "0");
        assert!(!env_truthy("GEMINI_TEST_TRUTHY"));
    }

    #[test]
    fn render_line_for_summary_formats_name_and_one_line() {
        let summary = RateLimitSummary {
            non_weekly_label: "5h".to_string(),
            non_weekly_remaining: 94,
            non_weekly_reset_epoch: Some(1700003600),
            weekly_remaining: 88,
            weekly_reset_epoch: 1700600000,
        };
        assert_eq!(
            render_line_for_summary("alpha", &summary, false, "%m-%d %H:%M"),
            "5h:94% W:88% 11-21 20:53"
        );
        assert_eq!(
            render_line_for_summary("alpha", &summary, true, "%m-%d %H:%M"),
            "alpha 5h:94% W:88% 11-21 20:53"
        );
    }

    #[test]
    fn json_helpers_escape_and_build_structures() {
        assert_eq!(json_escape("a\"b\\n"), "a\\\"b\\\\n");
        assert_eq!(
            json_array(vec![json_string("a"), json_string("b")]),
            "[\"a\",\"b\"]"
        );
        assert_eq!(
            json_obj(vec![
                ("k1".to_string(), json_string("v1")),
                ("k2".to_string(), "2".to_string())
            ]),
            "{\"k1\":\"v1\",\"k2\":2}"
        );
    }

    #[test]
    fn rate_limit_summary_to_json_includes_null_non_weekly_reset() {
        let summary = RateLimitSummary {
            non_weekly_label: "5h".to_string(),
            non_weekly_remaining: 90,
            non_weekly_reset_epoch: None,
            weekly_remaining: 80,
            weekly_reset_epoch: 1700600000,
        };
        let rendered = summary.to_json();
        assert!(rendered.contains("\"non_weekly_reset_epoch\":null"));
        assert!(rendered.contains("\"weekly_reset_epoch\":1700600000"));
    }

    #[test]
    fn json_result_item_to_json_supports_error_and_raw_usage_variants() {
        let item = JsonResultItem {
            name: "alpha".to_string(),
            target_file: "alpha.json".to_string(),
            status: "error".to_string(),
            ok: false,
            source: "network".to_string(),
            summary: None,
            raw_usage: Some("{\"rate_limit\":{}}".to_string()),
            error_code: Some("request-failed".to_string()),
            error_message: Some("boom".to_string()),
        };
        let rendered = item.to_json();
        assert!(rendered.contains("\"raw_usage\":{\"rate_limit\":{}}"));
        assert!(rendered.contains("\"error\":{\"code\":\"request-failed\",\"message\":\"boom\"}"));
    }

    #[test]
    fn collect_secret_files_returns_sorted_json_files() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secrets = dir.path().join("secrets");
        std::fs::create_dir_all(&secrets).expect("secrets");
        std::fs::write(secrets.join("b.json"), "{}").expect("b");
        std::fs::write(secrets.join("a.json"), "{}").expect("a");
        std::fs::write(secrets.join("skip.txt"), "x").expect("skip");
        let _secret = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);

        let files = collect_secret_files().expect("files");
        assert_eq!(
            files
                .iter()
                .map(|p| p.file_name().and_then(|v| v.to_str()).unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["a.json", "b.json"]
        );
    }

    #[test]
    fn resolve_single_target_appends_json_when_secret_dir_is_configured() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secrets = dir.path().join("secrets");
        std::fs::create_dir_all(&secrets).expect("secrets");
        let target = secrets.join("alpha.json");
        std::fs::write(&target, "{}").expect("target");
        let _secret = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);

        let resolved = resolve_single_target(Some("alpha")).expect("resolved");
        assert_eq!(resolved, target);
    }

    #[test]
    fn clear_starship_cache_rejects_non_absolute_cache_root() {
        let _cache = EnvGuard::set("ZSH_CACHE_DIR", "relative-cache");
        let err = clear_starship_cache().expect_err("non-absolute should fail");
        assert!(err.contains("non-absolute cache root"));
    }

    #[test]
    fn emit_helpers_cover_single_collection_and_error_envelopes() {
        let item = JsonResultItem {
            name: "alpha".to_string(),
            target_file: "alpha.json".to_string(),
            status: "ok".to_string(),
            ok: true,
            source: "network".to_string(),
            summary: Some(RateLimitSummary {
                non_weekly_label: "5h".to_string(),
                non_weekly_remaining: 94,
                non_weekly_reset_epoch: Some(1700003600),
                weekly_remaining: 88,
                weekly_reset_epoch: 1700600000,
            }),
            raw_usage: Some("{\"rate_limit\":{}}".to_string()),
            error_code: None,
            error_message: None,
        };
        emit_single_envelope("single", true, &item);
        emit_collection_envelope("all", true, &[item]);
        emit_error_json("failure", "boom", None);
    }
}
