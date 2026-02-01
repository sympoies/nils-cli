use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub struct RefreshLock {
    dir: PathBuf,
}

impl RefreshLock {
    pub fn acquire(dir: &Path, stale_seconds: u64) -> Option<Self> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        match std::fs::create_dir(dir) {
            Ok(()) => {
                return Some(Self {
                    dir: dir.to_path_buf(),
                })
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }

        if !is_stale(dir, stale_seconds) {
            return None;
        }

        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir(dir).ok()?;
        Some(Self {
            dir: dir.to_path_buf(),
        })
    }
}

impl Drop for RefreshLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn lock_dir_for_cache_file(cache_file: &Path) -> Option<PathBuf> {
    let stem = cache_file.file_stem()?.to_string_lossy();
    Some(cache_file.with_file_name(format!("{stem}.refresh.lock")))
}

fn is_stale_modified(
    modified: std::io::Result<SystemTime>,
    stale_seconds: u64,
    now: SystemTime,
) -> bool {
    let modified = match modified {
        Ok(value) => value,
        Err(_) => return true,
    };
    let age = match now.duration_since(modified) {
        Ok(value) => value,
        Err(_) => Duration::from_secs(0),
    };
    age.as_secs() >= stale_seconds
}

pub fn is_stale(dir: &Path, stale_seconds: u64) -> bool {
    if stale_seconds == 0 {
        return true;
    }

    let meta = match std::fs::metadata(dir) {
        Ok(value) => value,
        Err(_) => return true,
    };
    is_stale_modified(meta.modified(), stale_seconds, SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::io;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn refresh_lock_acquire_creates_and_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_dir = tmp.path().join("nested").join("usage.refresh.lock");

        let lock = RefreshLock::acquire(&lock_dir, 60).expect("acquire");
        assert!(lock_dir.is_dir());

        drop(lock);
        assert!(!lock_dir.exists());
    }

    #[test]
    fn refresh_lock_acquire_returns_none_when_lock_exists_and_not_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_dir = tmp.path().join("usage.refresh.lock");
        std::fs::create_dir(&lock_dir).unwrap();

        assert!(RefreshLock::acquire(&lock_dir, 3600).is_none());
        assert!(lock_dir.is_dir());
    }

    #[test]
    fn refresh_lock_acquire_reclaims_lock_when_stale_seconds_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_dir = tmp.path().join("usage.refresh.lock");
        std::fs::create_dir(&lock_dir).unwrap();
        let marker = lock_dir.join("marker");
        std::fs::write(&marker, b"marker").unwrap();
        assert!(marker.is_file());

        let lock = RefreshLock::acquire(&lock_dir, 0).expect("acquire");
        assert!(lock_dir.is_dir());
        assert!(!marker.exists());

        drop(lock);
        assert!(!lock_dir.exists());
    }

    #[test]
    fn refresh_lock_acquire_returns_none_on_create_dir_error() {
        let tmp = tempfile::tempdir().unwrap();
        let parent_file = tmp.path().join("not_a_dir");
        std::fs::write(&parent_file, b"not a dir").unwrap();

        let lock_dir = parent_file.join("usage.refresh.lock");
        assert!(RefreshLock::acquire(&lock_dir, 60).is_none());
    }

    #[test]
    fn lock_dir_for_cache_file_returns_none_when_stem_missing() {
        assert!(lock_dir_for_cache_file(Path::new("")).is_none());
    }

    #[test]
    fn lock_dir_for_cache_file_appends_refresh_lock_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_file = tmp.path().join("usage.json");

        let lock_dir = lock_dir_for_cache_file(&cache_file).expect("lock dir");
        assert_eq!(lock_dir, tmp.path().join("usage.refresh.lock"));
    }

    #[test]
    fn is_stale_returns_true_when_stale_seconds_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_dir = tmp.path().join("usage.refresh.lock");
        assert!(is_stale(&lock_dir, 0));
    }

    #[test]
    fn is_stale_returns_true_when_metadata_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing.refresh.lock");
        assert!(is_stale(&missing, 60));
    }

    #[test]
    fn is_stale_modified_returns_true_when_modified_fails() {
        let err = io::Error::other("boom");
        assert!(is_stale_modified(Err(err), 60, SystemTime::now()));
    }

    #[test]
    fn is_stale_modified_handles_future_modified_time() {
        let now = UNIX_EPOCH;
        let modified = UNIX_EPOCH + Duration::from_secs(10);
        assert!(!is_stale_modified(Ok(modified), 1, now));
    }

    #[test]
    fn is_stale_modified_returns_true_when_age_exceeds_threshold() {
        let now = UNIX_EPOCH + Duration::from_secs(100);
        let modified = UNIX_EPOCH + Duration::from_secs(90);
        assert!(is_stale_modified(Ok(modified), 5, now));
    }

    #[test]
    fn is_stale_modified_returns_false_when_age_below_threshold() {
        let now = UNIX_EPOCH + Duration::from_secs(100);
        let modified = UNIX_EPOCH + Duration::from_secs(99);
        assert!(!is_stale_modified(Ok(modified), 10, now));
    }
}
