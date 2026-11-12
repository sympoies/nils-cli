//! Per-target rate-limit cache: `<prompt-segment cache dir>/diag-rate-limits/<name>.kv`.
//!
//! The entry format and freshness policy are the shared `diag rate-limits`
//! ones. The cache never holds a token.

use super::scoped;
use nils_common::fs as shared_fs;
use nils_common::rate_limits::values as shared_values;
use nils_common::rate_limits::{CacheEntry, RateLimitWindow, WeeklyValues};
use nils_common::usage_cache_policy;
use serde_json::Value;
use std::io::ErrorKind;
use std::path::PathBuf;

/// Prefix of the shared `<PREFIX>_RATE_LIMITS_CACHE_*` settings.
const ENV_PREFIX: &str = "CLAUDE";
const CACHE_ALLOW_STALE_ENV: &str = "CLAUDE_RATE_LIMITS_CACHE_ALLOW_STALE";
#[cfg(test)]
const CACHE_TTL_ENV: &str = "CLAUDE_RATE_LIMITS_CACHE_TTL";

const LIMIT_RESETS_LINE_PREFIX: &str = "limit_resets=";

/// The `limit_resets` of the last network read, and when it was read.
pub struct StoredLimitResets {
    pub checked_at: i64,
    pub value: Value,
}

/// A cache entry with the extra data stored beside it, all from one read of
/// the file.
pub struct CacheRead {
    pub entry: CacheEntry,
    pub scoped: Vec<RateLimitWindow>,
    pub limit_resets: Option<StoredLimitResets>,
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

fn read_entry(name: &str, now_epoch: i64) -> Result<CacheRead, String> {
    let content = read_content(name)?;
    validate_entry(name, &content, now_epoch)
}

fn read_content(name: &str) -> Result<String, String> {
    let path = cache_file(name)?;
    std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "claude-rate-limits: cache not found (run claude-cli diag rate-limits without --cached to populate): {}",
            path.display()
        )
    })
}

fn validate_entry(name: &str, content: &str, now_epoch: i64) -> Result<CacheRead, String> {
    let path = cache_file(name)?;
    let entry = shared_values::parse_cache_entry(content);
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
    let stale = entry.is_stale(now_epoch, ttl_seconds());
    Ok(CacheRead {
        scoped: scoped::windows_from_cache(content, super::WEEKLY_MINUTES),
        limit_resets: limit_resets_from_cache(content),
        entry,
        stale,
    })
}

/// The entry `--cached` may show: within the display ceiling and the TTL.
pub fn read_for_cached_mode(name: &str, now_epoch: i64) -> Result<CacheRead, String> {
    let read = read_entry(name, now_epoch)?;
    if allow_stale() {
        return Ok(read);
    }
    if read.stale {
        let ttl = ttl_seconds();
        let age = read
            .entry
            .fetched_at_epoch
            .map(|fetched_at| now_epoch.saturating_sub(fetched_at).max(0))
            .unwrap_or_default();
        return Err(format!(
            "claude-rate-limits: cache expired (age={age}s, ttl={ttl}s): {} (rerun without --cached to refresh, or set {CACHE_ALLOW_STALE_ENV}=true)",
            cache_file(name)?.display()
        ));
    }
    Ok(read)
}

/// The entry a failed live read may fall back to, flagged when past the TTL.
pub fn read_allow_stale(name: &str, now_epoch: i64) -> Result<CacheRead, String> {
    read_entry(name, now_epoch)
}

/// The stale cache only when its atomically stored token digest matches.
pub fn read_allow_stale_owned(
    name: &str,
    token_key: &str,
    now_epoch: i64,
) -> Result<CacheRead, String> {
    let content = read_content(name)?;
    let owner = content
        .lines()
        .find_map(|line| line.strip_prefix("owner_token_key="));
    if owner != Some(token_key) {
        return Err("claude-rate-limits: cache belongs to a different account".to_string());
    }
    validate_entry(name, &content, now_epoch)
}

/// `limit_resets=<checked_at>,<json>`, written beside the windows it was read with.
fn limit_resets_cache_line(checked_at: i64, value: &Value) -> String {
    format!("\n{LIMIT_RESETS_LINE_PREFIX}{checked_at},{value}")
}

fn limit_resets_from_cache(content: &str) -> Option<StoredLimitResets> {
    let rest = content
        .lines()
        .find_map(|line| line.strip_prefix(LIMIT_RESETS_LINE_PREFIX))?;
    let (checked_at, value) = rest.split_once(',')?;
    Some(StoredLimitResets {
        checked_at: checked_at.parse().ok()?,
        value: serde_json::from_str(value).ok()?,
    })
}

/// Diagnostic age remains useful even when windows exceed the display ceiling.
pub(super) fn owned_age(name: &str, token_key: &str, now_epoch: i64) -> Option<i64> {
    let content = read_content(name).ok()?;
    if content
        .lines()
        .find_map(|line| line.strip_prefix("owner_token_key="))
        != Some(token_key)
    {
        return None;
    }
    let fetched_at = shared_values::parse_cache_entry(&content).fetched_at_epoch?;
    Some(now_epoch.saturating_sub(fetched_at).max(0))
}

/// Stores a network read: the windows and the `limit_resets` it carried.
/// `owner_key` binds the entry to the account that read it.
pub fn write(
    name: &str,
    fetched_at_epoch: i64,
    values: &WeeklyValues,
    windows: &[RateLimitWindow],
    limit_resets: &Value,
    owner_key: Option<&str>,
) -> Result<(), String> {
    let Some(mut data) = shared_values::render_cache_entry(fetched_at_epoch, values) else {
        return Ok(());
    };
    data.push_str(&scoped::cache_lines(windows));
    data.push_str(&limit_resets_cache_line(fetched_at_epoch, limit_resets));
    if let Some(owner_key) = owner_key {
        data.push_str(&format!("\nowner_token_key={owner_key}"));
    }
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
    use serde_json::json;

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
        write("alpha", 1_900_000_000, &values(), &[], &json!({}), None).unwrap();
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

    #[test]
    fn same_named_target_cache_is_rejected_after_account_switch() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let _dir = EnvGuard::set(
            &lock,
            "CLAUDE_PROMPT_SEGMENT_CACHE_DIR",
            tmp.path().to_str().unwrap(),
        );
        write(
            "active",
            1_900_000_000,
            &values(),
            &[],
            &json!({}),
            Some("token-account-a"),
        )
        .unwrap();

        assert!(read_allow_stale_owned("active", "token-account-a", 1_900_000_000).is_ok());
        assert!(read_allow_stale_owned("active", "token-account-b", 1_900_000_000).is_err());
    }
}
