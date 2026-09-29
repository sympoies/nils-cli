//! Per-target rate-limit cache: `<prompt-segment cache dir>/diag-rate-limits/<name>.kv`.
//!
//! The entry format and freshness policy are the shared `diag rate-limits`
//! ones. The cache never holds a token.

use nils_common::env as shared_env;
use nils_common::fs as shared_fs;
use nils_common::rate_limits::values as shared_values;
use nils_common::rate_limits::{CacheEntry, WeeklyValues};
use nils_common::usage_cache_policy;
use std::path::PathBuf;

const DEFAULT_CACHE_TTL_SECONDS: u64 = 180;
const CACHE_TTL_ENV: &str = "CLAUDE_RATE_LIMITS_CACHE_TTL";
const CACHE_ALLOW_STALE_ENV: &str = "CLAUDE_RATE_LIMITS_CACHE_ALLOW_STALE";

pub struct StaleCacheRead {
    pub entry: CacheEntry,
    pub stale: bool,
}

fn cache_file(name: &str) -> Result<PathBuf, String> {
    let dir = crate::prompt_segment::cache::cache_dir()
        .ok_or_else(|| "claude-rate-limits: cannot resolve the cache directory".to_string())?;
    Ok(dir.join("diag-rate-limits").join(format!("{name}.kv")))
}

fn read_entry(name: &str, now_epoch: i64) -> Result<CacheEntry, String> {
    let path = cache_file(name)?;
    let content = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "claude-rate-limits: cache not found (run claude-cli diag rate-limits without --cached to populate): {}",
            path.display()
        )
    })?;
    let entry = shared_values::parse_cache_entry(&content);
    if !entry.is_complete() {
        return Err(format!(
            "claude-rate-limits: invalid cache (incomplete window data): {}",
            path.display()
        ));
    }
    if !shared_values::fetched_at_within_display_age(entry.fetched_at_epoch, now_epoch) {
        return Err(format!(
            "claude-rate-limits: cache exceeds maximum display age (max={}s): {}",
            usage_cache_policy::MAX_DISPLAY_AGE_SECONDS,
            path.display()
        ));
    }
    Ok(entry)
}

/// The entry `--cached` may show: within the display ceiling and the TTL.
pub fn read_for_cached_mode(name: &str, now_epoch: i64) -> Result<CacheEntry, String> {
    let entry = read_entry(name, now_epoch)?;
    if shared_env::env_truthy_or(CACHE_ALLOW_STALE_ENV, false) {
        return Ok(entry);
    }
    let ttl = ttl_seconds();
    if entry.is_stale(now_epoch, ttl) {
        let age = entry
            .fetched_at_epoch
            .map(|fetched_at| now_epoch.saturating_sub(fetched_at).max(0))
            .unwrap_or_default();
        return Err(format!(
            "claude-rate-limits: cache expired (age={age}s, ttl={ttl}s): {} (rerun without --cached to refresh, or set {CACHE_ALLOW_STALE_ENV}=true)",
            cache_file(name)?.display()
        ));
    }
    Ok(entry)
}

/// The entry a failed live read may fall back to, flagged when past the TTL.
pub fn read_allow_stale(name: &str, now_epoch: i64) -> Result<StaleCacheRead, String> {
    let entry = read_entry(name, now_epoch)?;
    let stale = entry.is_stale(now_epoch, ttl_seconds());
    Ok(StaleCacheRead { entry, stale })
}

pub fn write(name: &str, fetched_at_epoch: i64, values: &WeeklyValues) -> Result<(), String> {
    let Some(data) = shared_values::render_cache_entry(fetched_at_epoch, values) else {
        return Ok(());
    };
    let path = cache_file(name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    shared_fs::write_atomic(&path, data.as_bytes(), shared_fs::SECRET_FILE_MODE)
        .map_err(|err| err.to_string())
}

fn ttl_seconds() -> u64 {
    std::env::var(CACHE_TTL_ENV)
        .ok()
        .and_then(|raw| shared_env::parse_duration_seconds(&raw))
        .unwrap_or(DEFAULT_CACHE_TTL_SECONDS)
}
