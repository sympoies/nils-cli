use std::path::Path;

use crate::auth;
use crate::paths;
use crate::rate_limits::cache;
use nils_common::env as shared_env;

mod lock;
mod refresh;
mod render;

pub use render::CacheEntry;

pub struct PromptSegmentOptions {
    pub no_5h: bool,
    pub ttl: Option<String>,
    pub time_format: Option<String>,
    pub show_timezone: bool,
    pub refresh: bool,
    pub is_enabled: bool,
}

const DEFAULT_TTL_SECONDS: u64 = 180;
const DEFAULT_TIME_FORMAT: &str = "%m-%d %H:%M";
const DEFAULT_TIME_FORMAT_WITH_TIMEZONE: &str = "%m-%d %H:%M %:z";

pub fn run(options: &PromptSegmentOptions) -> i32 {
    if options.is_enabled {
        return if prompt_segment_enabled() { 0 } else { 1 };
    }

    let ttl_seconds = match resolve_ttl_seconds(options.ttl.as_deref()) {
        Ok(value) => value,
        Err(_) => {
            print_ttl_usage();
            return 2;
        }
    };

    if !prompt_segment_enabled() {
        return 0;
    }

    let target_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => return 0,
    };

    let show_5h =
        shared_env::env_truthy_or("CODEX_PROMPT_SEGMENT_SHOW_5H_ENABLED", true) && !options.no_5h;
    let time_format = match options.time_format.as_deref() {
        Some(value) => value,
        None if options.show_timezone => DEFAULT_TIME_FORMAT_WITH_TIMEZONE,
        None => DEFAULT_TIME_FORMAT,
    };
    let stale_suffix = std::env::var("CODEX_PROMPT_SEGMENT_STALE_SUFFIX")
        .unwrap_or_else(|_| " (stale)".to_string());

    let prefix = resolve_name_prefix(&target_file);

    if options.refresh {
        if let Some(entry) = refresh::refresh_blocking(&target_file)
            && let Some(line) = render::render_line(&entry, &prefix, show_5h, time_format)
            && !line.trim().is_empty()
        {
            println!("{line}");
        }
        return 0;
    }

    let (cached, is_stale) = read_cached_entry(&target_file, ttl_seconds);
    if let Some(entry) = cached.clone()
        && let Some(mut line) = render::render_line(&entry, &prefix, show_5h, time_format)
    {
        if is_stale {
            line.push_str(&stale_suffix);
        }
        if !line.trim().is_empty() {
            println!("{line}");
        }
    }

    if cached.is_none() || is_stale {
        refresh::enqueue_background_refresh(&target_file);
    }

    0
}

fn prompt_segment_enabled() -> bool {
    shared_env::env_truthy("CODEX_PROMPT_SEGMENT_ENABLED")
}

fn resolve_ttl_seconds(cli_ttl: Option<&str>) -> Result<u64, ()> {
    if let Some(raw) = cli_ttl {
        return shared_env::parse_duration_seconds(raw).ok_or(());
    }

    if let Ok(raw) = std::env::var("CODEX_PROMPT_SEGMENT_TTL")
        && let Some(value) = shared_env::parse_duration_seconds(&raw)
    {
        return Ok(value);
    }

    Ok(DEFAULT_TTL_SECONDS)
}

fn print_ttl_usage() {
    eprintln!("codex-cli prompt-segment: invalid --ttl");
    eprintln!(
        "usage: codex-cli prompt-segment [--no-5h] [--ttl <duration>] [--time-format <strftime>] [--show-timezone] [--refresh] [--is-enabled]"
    );
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
    let name_source = std::env::var("CODEX_PROMPT_SEGMENT_NAME_SOURCE")
        .ok()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "secret".to_string());

    let show_fallback = shared_env::env_truthy("CODEX_PROMPT_SEGMENT_SHOW_FALLBACK_NAME_ENABLED");
    let show_full_email = shared_env::env_truthy("CODEX_PROMPT_SEGMENT_SHOW_FULL_EMAIL_ENABLED");

    if name_source == "email" {
        if let Ok(Some(email)) = auth::email_from_auth_file(target_file) {
            return Some(format_email_name(&email, show_full_email));
        }
        if show_fallback && let Ok(Some(identity)) = auth::identity_from_auth_file(target_file) {
            return Some(format_email_name(&identity, show_full_email));
        }
        return None;
    }

    if let Some(secret_name) = cache::secret_name_for_target(target_file) {
        return Some(secret_name);
    }

    if show_fallback && let Ok(Some(identity)) = auth::identity_from_auth_file(target_file) {
        return Some(format_email_name(&identity, show_full_email));
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
