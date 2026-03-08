use std::path::Path;

use nils_common::env as shared_env;

use crate::auth;
use crate::paths;
use crate::rate_limits;

mod refresh;
mod render;

pub use render::CacheEntry;

#[derive(Clone, Debug, Default)]
pub struct PromptSegmentOptions {
    pub no_5h: bool,
    pub ttl: Option<String>,
    pub time_format: Option<String>,
    pub show_timezone: bool,
    pub refresh: bool,
    pub is_enabled: bool,
}

const DEFAULT_TTL_SECONDS: u64 = 300;
const DEFAULT_TIME_FORMAT: &str = "%m-%d %H:%M";
const DEFAULT_TIME_FORMAT_WITH_TIMEZONE: &str = "%m-%d %H:%M %:z";
const DEFAULT_CODE_ASSIST_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
const DEFAULT_CODE_ASSIST_API_VERSION: &str = "v1internal";
const DEFAULT_CODE_ASSIST_PROJECT: &str = "projects/default";

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
    if !target_file.is_file() {
        return 0;
    }

    let show_5h =
        shared_env::env_truthy_or("GEMINI_PROMPT_SEGMENT_SHOW_5H_ENABLED", true) && !options.no_5h;
    let time_format = match options.time_format.as_deref() {
        Some(value) => value,
        None if options.show_timezone => DEFAULT_TIME_FORMAT_WITH_TIMEZONE,
        None => DEFAULT_TIME_FORMAT,
    };
    let stale_suffix = std::env::var("GEMINI_PROMPT_SEGMENT_STALE_SUFFIX")
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

fn read_cached_entry(target_file: &Path, ttl_seconds: u64) -> (Option<CacheEntry>, bool) {
    let cache_file = match rate_limits::cache_file_for_target(target_file) {
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

    let now = now_epoch();
    if now <= 0 || entry.fetched_at_epoch <= 0 {
        return (Some(entry), true);
    }

    let ttl = i64::try_from(ttl_seconds).unwrap_or(i64::MAX);
    let stale = now.saturating_sub(entry.fetched_at_epoch) > ttl;
    (Some(entry), stale)
}

fn resolve_ttl_seconds(cli_ttl: Option<&str>) -> Result<u64, ()> {
    if let Some(raw) = cli_ttl {
        return shared_env::parse_duration_seconds(raw).ok_or(());
    }

    if let Ok(raw) = std::env::var("GEMINI_PROMPT_SEGMENT_TTL")
        && let Some(value) = shared_env::parse_duration_seconds(&raw)
    {
        return Ok(value);
    }

    Ok(DEFAULT_TTL_SECONDS)
}

fn prompt_segment_enabled() -> bool {
    shared_env::env_truthy("GEMINI_PROMPT_SEGMENT_ENABLED")
}

fn print_ttl_usage() {
    eprintln!("gemini-cli prompt-segment: invalid --ttl");
    eprintln!(
        "usage: gemini-cli prompt-segment [--no-5h] [--ttl <duration>] [--time-format <strftime>] [--show-timezone] [--refresh] [--is-enabled]"
    );
}

fn resolve_name_prefix(target_file: &Path) -> String {
    let name = resolve_name(target_file);
    match name {
        Some(value) if !value.trim().is_empty() => format!("{} ", value.trim()),
        _ => String::new(),
    }
}

fn resolve_name(target_file: &Path) -> Option<String> {
    let source = std::env::var("GEMINI_PROMPT_SEGMENT_NAME_SOURCE")
        .ok()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "secret".to_string());
    let show_fallback = shared_env::env_truthy("GEMINI_PROMPT_SEGMENT_SHOW_FALLBACK_NAME_ENABLED");
    let show_full_email = shared_env::env_truthy("GEMINI_PROMPT_SEGMENT_SHOW_FULL_EMAIL_ENABLED");

    if source == "email" {
        if let Ok(Some(email)) = auth::email_from_auth_file(target_file) {
            return Some(format_email_name(&email, show_full_email));
        }
        if show_fallback && let Ok(Some(identity)) = auth::identity_from_auth_file(target_file) {
            return Some(format_email_name(&identity, show_full_email));
        }
        return None;
    }

    if let Some(secret_name) = rate_limits::secret_name_for_target(target_file) {
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

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn code_assist_endpoint() -> String {
    env_non_empty("CODE_ASSIST_ENDPOINT")
        .or_else(|| env_non_empty("GEMINI_CODE_ASSIST_ENDPOINT"))
        .unwrap_or_else(|| DEFAULT_CODE_ASSIST_ENDPOINT.to_string())
}

fn code_assist_api_version() -> String {
    env_non_empty("CODE_ASSIST_API_VERSION")
        .or_else(|| env_non_empty("GEMINI_CODE_ASSIST_API_VERSION"))
        .unwrap_or_else(|| DEFAULT_CODE_ASSIST_API_VERSION.to_string())
}

fn code_assist_project() -> String {
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

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}
