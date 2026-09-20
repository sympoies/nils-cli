use anyhow::{Context, Result};
use nils_common::env as shared_env;
use nils_common::fs as shared_fs;
use nils_common::usage_cache_policy;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const CACHE_FILE_NAME: &str = "usage.json";
#[derive(Clone, Copy, Debug)]
pub struct CacheSnapshot {
    exists: bool,
    modified: Option<SystemTime>,
    observed_at: SystemTime,
}

impl CacheSnapshot {
    pub fn exists(self) -> bool {
        self.exists
    }

    pub fn stale(self, ttl_seconds: u64) -> bool {
        if ttl_seconds == 0 || !self.exists {
            return true;
        }
        let Some(modified) = self.modified else {
            return true;
        };
        self.observed_at
            .duration_since(modified)
            .unwrap_or(Duration::ZERO)
            .as_secs()
            >= ttl_seconds
    }

    pub fn display_expired(self) -> bool {
        let Some(modified) = self.modified.filter(|_| self.exists) else {
            return true;
        };
        !usage_cache_policy::classify_display_age_seconds(signed_age_seconds(
            self.observed_at,
            modified,
        ))
        .is_display_eligible()
    }
}

pub fn cache_file() -> Option<PathBuf> {
    let dir = cache_dir()?;
    Some(dir.join(CACHE_FILE_NAME))
}

pub fn read_cache_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Removes the resolved usage cache file and the refresh throttle stamp.
///
/// Only the exact `<cache dir>/usage.json` and its `usage.refresh.at` sibling
/// are removed. The cache directory can be operator-supplied through
/// `CLAUDE_PROMPT_SEGMENT_CACHE_DIR`, so the directory itself is never deleted,
/// and neither are the refresh locks: `refresh.lock` and `refresh.spawn.lock`
/// belong to a possibly running background refresh, and unlinking a held lock
/// would let a second refresh acquire a fresh file and defeat the coalescing.
///
/// The throttle stamp *is* removed, because leaving it behind would suppress
/// the next background refresh for up to
/// `CLAUDE_PROMPT_SEGMENT_REFRESH_MIN_SECONDS` — an explicit clear would then
/// leave the prompt with no cache and no way to repopulate it.
///
/// Returns `Ok(false)` when there was no cache file to remove.
pub fn clear_usage_cache() -> Result<bool> {
    let Some(path) = cache_file() else {
        anyhow::bail!("claude-cli usage: cannot resolve the usage cache path");
    };
    clear_usage_cache_at(&path)
}

fn clear_usage_cache_at(path: &Path) -> Result<bool> {
    // `cache_dir` returns `CLAUDE_PROMPT_SEGMENT_CACHE_DIR` verbatim, and every
    // other cache operation accepts a relative value. Resolve against the
    // working directory so a clear behaves like the reads and writes beside it
    // rather than failing permanently; the file-name check below is what bounds
    // what may be deleted.
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .with_context(|| {
                format!(
                    "claude-cli usage: cannot resolve the relative cache path: {}",
                    path.display()
                )
            })?
            .join(path)
    };

    if resolved.file_name() != Some(std::ffi::OsStr::new(CACHE_FILE_NAME)) {
        anyhow::bail!(
            "claude-cli usage: refusing to clear an unexpected cache file: {}",
            resolved.display()
        );
    }

    let removed = match std::fs::remove_file(&resolved) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "claude-cli usage: failed to clear cache: {}",
                    resolved.display()
                )
            });
        }
    };

    clear_refresh_stamp(&resolved)?;

    Ok(removed)
}

/// Removes `<stem>.refresh.at` beside the cleared cache file.
///
/// The stamp name mirrors `refresh::sibling_path`. A missing stamp is success;
/// it only means no refresh has been attempted yet.
fn clear_refresh_stamp(cache_file: &Path) -> Result<()> {
    let Some(stem) = cache_file.file_stem() else {
        return Ok(());
    };
    let stamp = cache_file.with_file_name(format!("{}.refresh.at", stem.to_string_lossy()));
    match std::fs::remove_file(&stamp) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "claude-cli usage: failed to clear the refresh stamp: {}",
                stamp.display()
            )
        }),
    }
}

pub fn write_cache_file(path: &Path, body: &str) -> Result<()> {
    shared_fs::write_atomic(path, body.as_bytes(), shared_fs::SECRET_FILE_MODE)
        .with_context(|| format!("failed to write cache: {}", path.display()))
}

pub fn snapshot(path: &Path) -> CacheSnapshot {
    snapshot_at(path, SystemTime::now())
}

fn snapshot_at(path: &Path, observed_at: SystemTime) -> CacheSnapshot {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => CacheSnapshot {
            exists: true,
            modified: metadata.modified().ok(),
            observed_at,
        },
        _ => CacheSnapshot {
            exists: false,
            modified: None,
            observed_at,
        },
    }
}

pub fn cache_display_expired(path: &Path) -> bool {
    cache_display_expired_at(path, SystemTime::now())
}

fn cache_display_expired_at(path: &Path, now: SystemTime) -> bool {
    snapshot_at(path, now).display_expired()
}

fn signed_age_seconds(now: SystemTime, modified: SystemTime) -> Option<i64> {
    match now.duration_since(modified) {
        Ok(age) => i64::try_from(age.as_secs()).ok(),
        Err(_) => {
            let ahead = modified.duration_since(now).ok()?;
            let whole_seconds = i64::try_from(ahead.as_secs()).ok()?;
            let rounded_seconds = whole_seconds.checked_add(i64::from(ahead.subsec_nanos() > 0))?;
            rounded_seconds.checked_neg()
        }
    }
}

fn cache_dir() -> Option<PathBuf> {
    if let Some(value) = shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_CACHE_DIR") {
        return Some(PathBuf::from(value));
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    if cfg!(target_os = "macos") {
        Some(
            home.join("Library")
                .join("Caches")
                .join("claude-prompt-segment"),
        )
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from) {
        Some(xdg.join("claude-prompt-segment"))
    } else {
        Some(home.join(".cache").join("claude-prompt-segment"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cache_display_expired_at, clear_usage_cache_at, signed_age_seconds, snapshot, snapshot_at,
    };
    use pretty_assertions::assert_eq;
    use std::fs::File;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn cache_stale_treats_zero_ttl_as_stale() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");
        std::fs::write(&path, "{}").expect("write");
        assert!(snapshot(&path).stale(0));
    }

    #[test]
    fn cache_stale_uses_file_mtime() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");
        std::fs::write(&path, "{}").expect("write");
        let file = File::options().write(true).open(&path).expect("open");
        file.set_modified(SystemTime::now() - Duration::from_secs(120))
            .expect("set modified");

        assert!(snapshot(&path).stale(60));
        assert!(!snapshot(&path).stale(180));
    }

    #[test]
    fn cache_display_expiry_starts_at_600_seconds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");
        std::fs::write(&path, "{}").expect("write");
        let modified = UNIX_EPOCH + Duration::from_secs(1_000);
        let file = File::options().write(true).open(&path).expect("open");
        file.set_modified(modified).expect("set modified");

        assert!(!cache_display_expired_at(
            &path,
            modified + Duration::from_secs(599)
        ));
        assert!(cache_display_expired_at(
            &path,
            modified + Duration::from_secs(600)
        ));
    }

    #[test]
    fn cache_snapshot_reuses_one_observation_for_ttl_and_display_policy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");
        std::fs::write(&path, "{}").expect("write");
        let modified = UNIX_EPOCH + Duration::from_secs(1_000);
        let file = File::options().write(true).open(&path).expect("open");
        file.set_modified(modified).expect("set modified");

        let eligible = snapshot_at(&path, modified + Duration::from_secs(599));
        assert!(eligible.exists());
        assert!(eligible.stale(60));
        assert!(!eligible.display_expired());

        let expired = snapshot_at(&path, modified + Duration::from_secs(600));
        assert!(expired.stale(60));
        assert!(expired.display_expired());
    }

    #[test]
    fn cache_display_future_clock_tolerance_ends_after_5_seconds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");
        std::fs::write(&path, "{}").expect("write");
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let file = File::options().write(true).open(&path).expect("open");

        file.set_modified(now + Duration::from_secs(5))
            .expect("set modified within tolerance");
        assert!(!cache_display_expired_at(&path, now));

        file.set_modified(now + Duration::from_secs(6))
            .expect("set modified beyond tolerance");
        assert!(cache_display_expired_at(&path, now));
    }

    #[test]
    fn cache_display_future_age_conversion_rounds_away_from_zero() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);

        assert_eq!(
            signed_age_seconds(now, now + Duration::from_secs(5)),
            Some(-5)
        );
        assert_eq!(
            signed_age_seconds(now, now + Duration::from_secs(5) + Duration::from_nanos(1)),
            Some(-6)
        );
    }

    #[test]
    fn clear_usage_cache_removes_the_cache_and_the_refresh_stamp_only() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");
        let lock = tmp.path().join("usage.refresh.lock");
        let spawn_lock = tmp.path().join("usage.refresh.spawn.lock");
        let stamp = tmp.path().join("usage.refresh.at");
        std::fs::write(&path, "{}").expect("write cache");
        std::fs::write(&lock, "").expect("write lock");
        std::fs::write(&spawn_lock, "").expect("write spawn lock");
        std::fs::write(&stamp, "1").expect("write refresh stamp");

        assert!(clear_usage_cache_at(&path).expect("clear"));
        assert!(!path.exists());
        assert!(
            !stamp.exists(),
            "the refresh throttle stamp must not survive a clear, or the next \
             background refresh stays suppressed with no cache to render"
        );
        assert!(lock.is_file(), "refresh locks must survive a cache clear");
        assert!(
            spawn_lock.is_file(),
            "spawn locks must survive a cache clear"
        );
        assert!(tmp.path().is_dir(), "cache dir must survive a cache clear");
    }

    #[test]
    fn clear_usage_cache_is_a_quiet_success_when_the_cache_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");

        assert!(!clear_usage_cache_at(&path).expect("clear"));
    }

    #[test]
    fn clear_usage_cache_rejects_an_unexpected_cache_file_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("credentials.json");
        std::fs::write(&path, "{}").expect("write");

        let err = clear_usage_cache_at(&path).expect_err("unexpected file name should fail");
        assert!(err.to_string().contains("unexpected cache file"));
        assert!(path.is_file());
    }

    #[test]
    fn clear_usage_cache_resolves_a_relative_cache_path_like_the_other_operations() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("usage.json");
        std::fs::write(&path, "{}").expect("write cache");

        // A relative `CLAUDE_PROMPT_SEGMENT_CACHE_DIR` is accepted by
        // `read_cache_file` and `write_cache_file`, so a clear resolves it the
        // same way instead of failing permanently.
        let relative = std::path::Path::new("usage.json");
        assert!(!relative.is_absolute());

        let resolved_elsewhere = clear_usage_cache_at(relative).expect("relative clear");
        assert!(
            !resolved_elsewhere,
            "a relative path resolves against the working directory, which holds no cache here"
        );
        assert!(path.is_file(), "the fixture cache is a different directory");
    }

    #[test]
    fn clear_usage_cache_still_refuses_an_unexpected_relative_file_name() {
        let err = clear_usage_cache_at(std::path::Path::new("credentials.json"))
            .expect_err("unexpected relative file name should fail");
        assert!(err.to_string().contains("unexpected cache file"));
    }
}
