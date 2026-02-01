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

pub fn is_stale(dir: &Path, stale_seconds: u64) -> bool {
    if stale_seconds == 0 {
        return true;
    }

    let meta = match std::fs::metadata(dir) {
        Ok(value) => value,
        Err(_) => return true,
    };
    let modified = match meta.modified() {
        Ok(value) => value,
        Err(_) => return true,
    };
    let now = SystemTime::now();
    let age = match now.duration_since(modified) {
        Ok(value) => value,
        Err(_) => Duration::from_secs(0),
    };
    age.as_secs() >= stale_seconds
}
