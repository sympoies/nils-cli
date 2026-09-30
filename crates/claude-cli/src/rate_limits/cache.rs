//! Per-target rate-limit cache: `<prompt-segment cache dir>/diag-rate-limits/<name>.kv`.
//!
//! The entry format and freshness policy are the shared `diag rate-limits`
//! ones. The cache never holds a token.

use nils_common::fs as shared_fs;
use nils_common::rate_limits::values as shared_values;
use nils_common::rate_limits::{CacheEntry, WeeklyValues};
use nils_common::usage_cache_policy;
use std::io::ErrorKind;
use std::path::PathBuf;

/// Prefix of the shared `<PREFIX>_RATE_LIMITS_CACHE_*` settings.
const ENV_PREFIX: &str = "CLAUDE";
const CACHE_ALLOW_STALE_ENV: &str = "CLAUDE_RATE_LIMITS_CACHE_ALLOW_STALE";
#[cfg(test)]
const CACHE_TTL_ENV: &str = "CLAUDE_RATE_LIMITS_CACHE_TTL";

pub struct StaleCacheRead {
    pub entry: CacheEntry,
    pub stale: bool,
}

fn entries_dir() -> Result<PathBuf, String> {
    let dir = crate::prompt_segment::cache::cache_dir()
        .ok_or_else(|| "claude-rate-limits: cannot resolve the cache directory".to_string())?;
    Ok(dir.join("diag-rate-limits"))
}

fn cache_file(name: &str) -> Result<PathBuf, String> {
    Ok(entries_dir()?.join(format!("{name}.kv")))
}

/// Removes every cached `diag rate-limits` entry for `-c`, like Codex does.
/// Other prompt-segment cache files are left alone.
pub fn clear() -> Result<(), String> {
    let dir = entries_dir()?;
    if !dir.is_absolute() {
        return Err(format!(
            "claude-rate-limits: refusing to clear cache with non-absolute cache dir: {}",
            dir.display()
        ));
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "claude-rate-limits: failed to clear cache {}: {err}",
            dir.display()
        )),
    }
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
    if allow_stale() {
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
    shared_values::cache_ttl_seconds(ENV_PREFIX)
}

fn allow_stale() -> bool {
    shared_values::cache_allow_stale(ENV_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_common::rate_limits::WindowValues;
    use nils_test_support::{EnvGuard, GlobalStateLock};

    fn values() -> WeeklyValues {
        WeeklyValues {
            weekly: Some(WindowValues {
                label: "Weekly".to_string(),
                remaining: 60,
                reset_epoch: 2_000_000_000,
            }),
            non_weekly: Some(WindowValues {
                label: "5h".to_string(),
                remaining: 80,
                reset_epoch: 2_000_000_000,
            }),
        }
    }

    #[test]
    fn clear_removes_every_cached_entry_and_tolerates_a_missing_cache() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let _dir = EnvGuard::set(
            &lock,
            "CLAUDE_PROMPT_SEGMENT_CACHE_DIR",
            tmp.path().to_str().unwrap(),
        );
        write("alpha", 1_900_000_000, &values()).unwrap();
        assert!(cache_file("alpha").unwrap().is_file());
        let unrelated = tmp.path().join("prompt-segment.kv");
        std::fs::write(&unrelated, "kept").unwrap();

        clear().unwrap();

        assert!(!cache_file("alpha").unwrap().exists());
        assert!(unrelated.is_file(), "only the diag cache is cleared");
        clear().unwrap();
    }

    #[test]
    fn ttl_and_allow_stale_follow_the_claude_environment() {
        let lock = GlobalStateLock::new();
        let _ttl = EnvGuard::set(&lock, CACHE_TTL_ENV, "5m");
        let _stale = EnvGuard::set(&lock, CACHE_ALLOW_STALE_ENV, "true");
        assert_eq!(ttl_seconds(), 300);
        assert!(allow_stale());
    }
}
