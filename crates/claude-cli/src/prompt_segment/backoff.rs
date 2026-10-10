//! Same-host usage cooldown, shared across processes and caller types.
//! Only a token digest and retry timing are persisted; clearing usage caches
//! does not clear this state. The lock spans the eligibility check and request.
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nils_common::fs as shared_fs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::cache;

const DEFAULT_DELAY: u64 = 300;
const MAX_DELAY: u64 = 3600;

#[derive(Deserialize, Serialize)]
struct State {
    retry_at: u64,
    delay_seconds: u64,
}

pub(crate) struct Backoff {
    _lock: File,
    path: PathBuf,
}

impl Backoff {
    pub(crate) fn acquire(token: &str) -> std::io::Result<Self> {
        let path = state_path(token).ok_or_else(|| std::io::Error::other("cache unavailable"))?;
        let dir = path.parent().expect("state parent");
        std::fs::create_dir_all(dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path.with_extension("lock"))?;
        if !lock.metadata()?.is_file() {
            return Err(std::io::Error::other("invalid usage lock"));
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { _lock: lock, path })
    }

    fn read(&self) -> Option<State> {
        serde_json::from_slice(&std::fs::read(&self.path).ok()?).ok()
    }

    pub(crate) fn active(&self, now: u64) -> bool {
        self.read().is_some_and(|state| state.retry_at > now)
    }

    pub(crate) fn record(&self, now: u64, retry_after: Option<u64>) -> std::io::Result<()> {
        let previous = self.read().map(|state| state.delay_seconds);
        let delay_seconds = next_delay(previous, retry_after);
        let state = State {
            retry_at: now.saturating_add(delay_seconds),
            delay_seconds,
        };
        shared_fs::write_atomic(
            &self.path,
            &serde_json::to_vec(&state)?,
            shared_fs::SECRET_FILE_MODE,
        )
        .map_err(|_| std::io::Error::other("backoff write failed"))
    }

    pub(crate) fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

pub(crate) fn token_key(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn state_path(token: &str) -> Option<PathBuf> {
    Some(
        cache::cache_dir()?
            .join("usage-backoff")
            .join(format!("{}.json", token_key(token))),
    )
}

fn active_path(path: Option<PathBuf>) -> bool {
    path.and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<State>(&bytes).ok())
        .is_some_and(|state| state.retry_at > now_epoch())
}

pub(crate) fn active_for(token: &str) -> bool {
    active_path(state_path(token))
}

/// Bind the single prompt cache to its OAuth reader without storing a token.
/// Foreground prompt rendering can then inspect cooldown without Keychain I/O.
pub(crate) fn bind_prompt_token(token: &str) {
    if let Some(dir) = cache::cache_dir() {
        let _ = shared_fs::write_atomic(
            &dir.join("usage.account"),
            token_key(token).as_bytes(),
            shared_fs::SECRET_FILE_MODE,
        );
    }
}

pub(crate) fn prompt_key() -> Option<String> {
    let key = std::fs::read_to_string(cache::cache_dir()?.join("usage.account")).ok()?;
    (key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(key)
}

pub(crate) fn prompt_snapshot() -> (Option<String>, bool) {
    let key = prompt_key();
    let active = active_path(cache::cache_dir().and_then(|dir| {
        Some(
            dir.join("usage-backoff")
                .join(format!("{}.json", key.as_ref()?)),
        )
    }));
    (key, active)
}

pub(crate) fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn next_delay(previous: Option<u64>, retry_after: Option<u64>) -> u64 {
    retry_after
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_DELAY)
        .max(previous.unwrap_or(0).saturating_mul(2))
        .min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn fake_clock_tracks_consecutive_limits_and_success_per_token() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::tempdir().unwrap();
        let _cache = EnvGuard::set(
            &lock,
            "CLAUDE_PROMPT_SEGMENT_CACHE_DIR",
            tmp.path().to_str().unwrap(),
        );
        let state = Backoff::acquire("fixture-alpha").unwrap();
        state.record(1000, Some(0)).unwrap();
        assert!(state.active(1299));
        assert!(!state.active(1300));
        let other = Backoff::acquire("fixture-beta").unwrap();
        assert!(!other.active(1000));
        state.record(1300, None).unwrap();
        assert!(state.active(1899));
        assert!(!state.active(1900));
        state.clear().unwrap();
        state.record(2000, None).unwrap();
        assert!(!state.active(2300), "success resets the consecutive delay");
    }

    #[test]
    fn prompt_snapshot_keeps_account_and_cooldown_together() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::tempdir().unwrap();
        let _cache = EnvGuard::set(
            &lock,
            "CLAUDE_PROMPT_SEGMENT_CACHE_DIR",
            tmp.path().to_str().unwrap(),
        );
        bind_prompt_token("fixture-alpha");
        let snapshot = prompt_snapshot();
        let other = Backoff::acquire("fixture-beta").unwrap();
        other.record(now_epoch(), None).unwrap();
        bind_prompt_token("fixture-beta");
        assert_eq!(snapshot, (Some(token_key("fixture-alpha")), false));
        assert_eq!(prompt_snapshot(), (Some(token_key("fixture-beta")), true));
    }

    #[test]
    fn retry_delay_honors_positive_header_and_caps_consecutive_limits() {
        assert_eq!(next_delay(None, Some(60)), 60);
        assert_eq!(next_delay(None, Some(0)), 300);
        assert_eq!(next_delay(Some(300), None), 600);
        assert_eq!(next_delay(Some(600), Some(1200)), 1200);
        assert_eq!(next_delay(Some(2400), None), 3600);
        assert_eq!(next_delay(Some(u64::MAX), None), 3600);
        assert_eq!(next_delay(None, Some(7200)), 3600);
    }
}
