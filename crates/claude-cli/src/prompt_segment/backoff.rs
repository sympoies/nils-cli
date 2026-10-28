//! Same-host usage cooldown, shared across processes and caller types.
//! Account digests, usage snapshots and retry timing are persisted without
//! credentials. The lock spans eligibility, request and snapshot publication.
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nils_common::fs as shared_fs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::cache;

mod account;

const FETCH_INTERVAL: u64 = 60;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct UsageObservation {
    pub(crate) fetched_at: i64,
    pub(crate) body: String,
}

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
        let (account_key, aliases) = account::resolve(token);
        let path = state_path_for_key(&account_key)
            .ok_or_else(|| std::io::Error::other("cache unavailable"))?;
        let dir = path.parent().expect("state parent").to_path_buf();
        std::fs::create_dir_all(&dir)?;
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
        let state = Self { _lock: lock, path };
        // Import all known aliases while the account lock is held, before the
        // first reader of any alias can decide whether HTTP is eligible.
        for token_key in aliases {
            let token_state_path = dir.join(format!("{token_key}.json"));
            if token_state_path != state.path
                && let Ok(bytes) = std::fs::read(&token_state_path)
                && let Ok(token_state) = serde_json::from_slice::<State>(&bytes)
                && token_state.retry_at > now_epoch()
                && token_state.retry_at > state.read().map(|state| state.retry_at).unwrap_or(0)
            {
                shared_fs::write_atomic(&state.path, &bytes, shared_fs::SECRET_FILE_MODE)
                    .map_err(|_| std::io::Error::other("backoff migration failed"))?;
                let _ = std::fs::remove_file(token_state_path);
            }
        }
        Ok(state)
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

    pub(crate) fn cached_response(&self, now: u64) -> Option<UsageObservation> {
        let response = read_response(&self.path)?;
        let age = i64::try_from(now).ok()?.checked_sub(response.fetched_at)?;
        (0..FETCH_INTERVAL as i64)
            .contains(&age)
            .then_some(response)
    }

    pub(crate) fn store_response(&self, response: &UsageObservation) -> std::io::Result<()> {
        shared_fs::write_atomic(
            &self.path.with_extension("response"),
            &serde_json::to_vec(response)?,
            shared_fs::SECRET_FILE_MODE,
        )
        .map_err(|_| std::io::Error::other("usage cache write failed"))
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
    state_path_for_key(&account::key(token))
}

fn state_path_for_key(key: &str) -> Option<PathBuf> {
    Some(
        cache::cache_dir()?
            .join("usage-backoff")
            .join(format!("{key}.json")),
    )
}

fn read_response(path: &std::path::Path) -> Option<UsageObservation> {
    serde_json::from_slice(&std::fs::read(path.with_extension("response")).ok()?).ok()
}

pub(crate) fn cached_usage(token: &str) -> Option<UsageObservation> {
    let response = read_response(&state_path(token)?)?;
    let age = i64::try_from(now_epoch())
        .ok()?
        .checked_sub(response.fetched_at)?;
    nils_common::usage_cache_policy::classify_display_age_seconds(Some(age))
        .is_display_eligible()
        .then_some(response)
}

pub(crate) fn fetched_at(token: &str) -> Option<i64> {
    read_response(&state_path(token)?).map(|response| response.fetched_at)
}

pub(crate) fn retry_at(token: &str) -> Option<u64> {
    let state: State = serde_json::from_slice(&std::fs::read(state_path(token)?).ok()?).ok()?;
    (state.retry_at > now_epoch()).then_some(state.retry_at)
}

pub(crate) fn invalidate_response(token: &str) {
    if let Ok(state) = Backoff::acquire(token) {
        let _ = std::fs::remove_file(state.path.with_extension("response"));
    }
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
            format!("{}\n{}", token_key(token), account::key(token)).as_bytes(),
            shared_fs::SECRET_FILE_MODE,
        );
    }
}

#[cfg(test)]
pub(crate) fn prompt_key() -> Option<String> {
    let key = std::fs::read_to_string(cache::cache_dir()?.join("usage.account")).ok()?;
    let key = key.lines().next()?.to_string();
    (key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(key)
}

pub(crate) fn prompt_snapshot() -> (Option<String>, bool) {
    let Some(dir) = cache::cache_dir() else {
        return (None, false);
    };
    let Some(binding) = std::fs::read_to_string(dir.join("usage.account")).ok() else {
        return (None, false);
    };
    let mut keys = binding.lines();
    let valid_key = |key: &str| key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit());
    let token_key = keys.next().filter(|key| valid_key(key));
    let account_key = keys.next().or(token_key).filter(|key| valid_key(key));
    let active =
        active_path(account_key.map(|key| dir.join("usage-backoff").join(format!("{key}.json"))));
    (token_key.map(str::to_string), active)
}

pub(crate) fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn next_delay(previous: Option<u64>, retry_after: Option<u64>) -> u64 {
    let server_delay = retry_after.filter(|seconds| *seconds > 0);
    server_delay
        .unwrap_or(DEFAULT_DELAY)
        .max(previous.unwrap_or(0).saturating_mul(2))
        .min(MAX_DELAY)
        .max(server_delay.unwrap_or(0))
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
    fn shared_response_interval_and_token_cooldown_keep_original_timing() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::tempdir().unwrap();
        let _cache = EnvGuard::set(
            &lock,
            "CLAUDE_PROMPT_SEGMENT_CACHE_DIR",
            tmp.path().to_str().unwrap(),
        );
        let _profiles = EnvGuard::set(&lock, "CLAUDE_SECRET_DIR", tmp.path().to_str().unwrap());
        std::fs::write(tmp.path().join("alpha.json"), r#"{"claudeAiOauth":{"accessToken":"fixture-alpha"},"oauthAccount":{"accountUuid":"fixture-account"}}"#).unwrap();
        let dir = tmp.path().join("usage-backoff");
        std::fs::create_dir_all(&dir).unwrap();
        let token_state = dir.join(format!("{}.json", token_key("fixture-alpha")));
        let retry_at = now_epoch() + 7200;
        std::fs::write(
            &token_state,
            serde_json::json!({"retry_at":retry_at,"delay_seconds":7200}).to_string(),
        )
        .unwrap();
        let state = Backoff::acquire("fixture-alpha").unwrap();
        assert!(state.active(retry_at - 1));
        assert!(!state.active(retry_at));
        state
            .store_response(&UsageObservation {
                fetched_at: 8200,
                body: "{}".to_string(),
            })
            .unwrap();
        assert_eq!(
            state.cached_response(8199).map(|response| response.body),
            None
        );
        assert_eq!(
            state
                .cached_response(8259)
                .map(|response| response.body)
                .as_deref(),
            Some("{}")
        );
        assert_eq!(
            state.cached_response(8260).map(|response| response.body),
            None
        );
        state.clear().unwrap();
        drop(state);
        std::fs::write(&token_state, r#"{"retry_at":1,"delay_seconds":3600}"#).unwrap();
        let state = Backoff::acquire("fixture-alpha").unwrap();
        state.record(8300, None).unwrap();
        assert!(
            !state.active(8600),
            "successful reads reset migrated consecutive delay"
        );
    }

    #[test]
    fn prompt_account_switch_rebinds_snapshot_to_the_activated_token() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::tempdir().unwrap();
        let _cache = EnvGuard::set(
            &lock,
            "CLAUDE_PROMPT_SEGMENT_CACHE_DIR",
            tmp.path().to_str().unwrap(),
        );
        bind_prompt_token("fixture-alpha");
        let snapshot = prompt_snapshot();
        assert_eq!(snapshot.0, Some(token_key("fixture-alpha")));
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
        assert_eq!(next_delay(None, Some(7200)), 7200);
    }
}
