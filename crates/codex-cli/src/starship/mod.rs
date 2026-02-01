use std::path::Path;

use crate::auth;
use crate::paths;
use crate::rate_limits::cache;

mod lock;
mod refresh;
mod render;

pub use render::CacheEntry;

pub struct StarshipOptions {
    pub no_5h: bool,
    pub ttl: Option<String>,
    pub time_format: Option<String>,
    pub refresh: bool,
    pub is_enabled: bool,
}

const DEFAULT_TTL_SECONDS: u64 = 300;
const DEFAULT_TIME_FORMAT: &str = "%m-%d %H:%M";

pub fn run(options: &StarshipOptions) -> i32 {
    if options.is_enabled {
        return if starship_enabled() { 0 } else { 1 };
    }

    let ttl_seconds = match resolve_ttl_seconds(options.ttl.as_deref()) {
        Ok(value) => value,
        Err(_) => {
            print_ttl_usage();
            return 2;
        }
    };

    if !starship_enabled() {
        return 0;
    }

    let target_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return 0,
    };

    let show_5h = env_truthy_default("CODEX_STARSHIP_SHOW_5H_ENABLED", true) && !options.no_5h;
    let time_format = options
        .time_format
        .as_deref()
        .unwrap_or(DEFAULT_TIME_FORMAT);
    let stale_suffix =
        std::env::var("CODEX_STARSHIP_STALE_SUFFIX").unwrap_or_else(|_| " (stale)".to_string());

    let prefix = resolve_name_prefix(&target_file);

    if options.refresh {
        if let Some(entry) = refresh::refresh_blocking(&target_file) {
            if let Some(line) = render::render_line(&entry, &prefix, show_5h, time_format) {
                if !line.trim().is_empty() {
                    println!("{line}");
                }
            }
        }
        return 0;
    }

    let (cached, is_stale) = read_cached_entry(&target_file, ttl_seconds);
    if let Some(entry) = cached.clone() {
        if let Some(mut line) = render::render_line(&entry, &prefix, show_5h, time_format) {
            if is_stale {
                line.push_str(&stale_suffix);
            }
            if !line.trim().is_empty() {
                println!("{line}");
            }
        }
    }

    if cached.is_none() || is_stale {
        refresh::enqueue_background_refresh(&target_file);
    }

    0
}

fn starship_enabled() -> bool {
    env_truthy("CODEX_STARSHIP_ENABLED")
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| matches!(value.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn env_truthy_default(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(value.to_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

fn resolve_ttl_seconds(cli_ttl: Option<&str>) -> Result<u64, ()> {
    if let Some(raw) = cli_ttl {
        return parse_duration_seconds(raw).ok_or(());
    }

    if let Ok(raw) = std::env::var("CODEX_STARSHIP_TTL") {
        if let Some(value) = parse_duration_seconds(&raw) {
            return Ok(value);
        }
    }

    Ok(DEFAULT_TTL_SECONDS)
}

fn parse_duration_seconds(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let raw = raw.to_ascii_lowercase();
    let (num_part, multiplier): (&str, u64) = match raw.chars().last()? {
        's' => (&raw[..raw.len().saturating_sub(1)], 1),
        'm' => (&raw[..raw.len().saturating_sub(1)], 60),
        'h' => (&raw[..raw.len().saturating_sub(1)], 60 * 60),
        'd' => (&raw[..raw.len().saturating_sub(1)], 60 * 60 * 24),
        'w' => (&raw[..raw.len().saturating_sub(1)], 60 * 60 * 24 * 7),
        ch if ch.is_ascii_digit() => (raw.as_str(), 1),
        _ => return None,
    };

    let num_part = num_part.trim();
    if num_part.is_empty() {
        return None;
    }

    let value = num_part.parse::<u64>().ok()?;
    if value == 0 {
        return None;
    }

    value.checked_mul(multiplier)
}

fn print_ttl_usage() {
    eprintln!("codex-cli starship: invalid --ttl");
    eprintln!("usage: codex-cli starship [--no-5h] [--ttl <duration>] [--time-format <strftime>] [--refresh] [--is-enabled]");
}

fn read_cached_entry(target_file: &Path, ttl_seconds: u64) -> (Option<CacheEntry>, bool) {
    let cache_file = match cache::cache_file_for_target(target_file) {
        Ok(value) => value,
        Err(_) => return (None, false),
    };
    if !cache_file.is_file() {
        return (None, false);
    }

    let entry = render::read_cache_file(&cache_file);
    let Some(entry) = entry else {
        return (None, false);
    };

    let now_epoch = chrono::Utc::now().timestamp();
    if now_epoch <= 0 || entry.fetched_at_epoch <= 0 {
        return (Some(entry), true);
    }

    let ttl_i64 = i64::try_from(ttl_seconds).unwrap_or(i64::MAX);
    let stale = now_epoch.saturating_sub(entry.fetched_at_epoch) > ttl_i64;
    (Some(entry), stale)
}

fn resolve_name_prefix(target_file: &Path) -> String {
    let name = resolve_name(target_file);
    match name {
        Some(value) if !value.trim().is_empty() => format!("{} ", value.trim()),
        _ => String::new(),
    }
}

fn resolve_name(target_file: &Path) -> Option<String> {
    let name_source = std::env::var("CODEX_STARSHIP_NAME_SOURCE")
        .ok()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "secret".to_string());

    let show_fallback = env_truthy("CODEX_STARSHIP_SHOW_FALLBACK_NAME_ENABLED");
    let show_full_email = env_truthy("CODEX_STARSHIP_SHOW_FULL_EMAIL_ENABLED");

    if name_source == "email" {
        if let Ok(Some(email)) = auth::email_from_auth_file(target_file) {
            return Some(format_email_name(&email, show_full_email));
        }
        if show_fallback {
            if let Ok(Some(identity)) = auth::identity_from_auth_file(target_file) {
                return Some(format_email_name(&identity, show_full_email));
            }
        }
        return None;
    }

    if let Some(secret_name) = cache::secret_name_for_target(target_file) {
        return Some(secret_name);
    }

    if show_fallback {
        if let Ok(Some(identity)) = auth::identity_from_auth_file(target_file) {
            return Some(format_email_name(&identity, show_full_email));
        }
    }

    None
}

fn format_email_name(raw: &str, show_full_email: bool) -> String {
    let trimmed = raw.trim();
    if show_full_email {
        return trimmed.to_string();
    }
    trimmed.split('@').next().unwrap_or(trimmed).to_string()
}
