use gemini_cli::rate_limits;

use std::ffi::{OsStr, OsString};
use std::fs as stdfs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    match LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: tests serialize env mutations via env_lock.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            // SAFETY: tests serialize env mutations via env_lock.
            unsafe { std::env::set_var(self.key, value) };
        } else {
            // SAFETY: tests serialize env mutations via env_lock.
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "nils-gemini-cli-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = stdfs::remove_dir_all(&path);
        stdfs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    fn join(&self, child: &str) -> PathBuf {
        self.path.join(child)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = stdfs::remove_dir_all(&self.path);
    }
}

fn write_secret(path: &Path, with_access_token: bool) {
    let payload = if with_access_token {
        r#"{"tokens":{"access_token":"tok","account_id":"acct_001"}}"#
    } else {
        r#"{"tokens":{"account_id":"acct_001"}}"#
    };
    stdfs::write(path, payload).expect("write secret");
}

#[test]
fn rate_limits_single_json_one_line_conflict_returns_64() {
    let _lock = env_lock();

    let options = rate_limits::RateLimitsOptions {
        json: true,
        one_line: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 64);
}

#[test]
fn rate_limits_single_cached_json_conflict_returns_64() {
    let _lock = env_lock();

    let options = rate_limits::RateLimitsOptions {
        cached: true,
        json: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 64);
}

#[test]
fn rate_limits_single_cached_clear_cache_conflict_returns_64() {
    let _lock = env_lock();

    let options = rate_limits::RateLimitsOptions {
        cached: true,
        clear_cache: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 64);
}

#[test]
fn rate_limits_single_json_target_not_found_returns_1() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-single-target-not-found");
    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    let secrets = stdfs::canonicalize(&secrets).expect("canonical secrets");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _default_all = EnvGuard::set("GEMINI_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false");

    let options = rate_limits::RateLimitsOptions {
        json: true,
        secret: Some("alpha.json".to_string()),
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 1);
}

#[test]
fn rate_limits_single_cached_missing_cache_returns_1() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-single-missing-cache");

    let auth_file = dir.join("auth.json");
    write_secret(&auth_file, true);
    let cache_root = dir.join("cache-root");
    stdfs::create_dir_all(&cache_root).expect("cache root");

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _default_all = EnvGuard::set("GEMINI_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false");

    let options = rate_limits::RateLimitsOptions {
        cached: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 1);
}

#[test]
fn rate_limits_single_cached_success_returns_0() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-single-cached-success");

    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    let secret_file = secrets.join("alpha.json");
    write_secret(&secret_file, true);
    let secrets = stdfs::canonicalize(&secrets).expect("canonical secrets");
    let secret_file = secrets.join("alpha.json");

    let cache_root = dir.join("cache-root");
    stdfs::create_dir_all(&cache_root).expect("cache root");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _default_all = EnvGuard::set("GEMINI_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false");

    let cache_file = rate_limits::cache_file_for_target(&secret_file).expect("cache path");
    if let Some(parent) = cache_file.parent() {
        stdfs::create_dir_all(parent).expect("cache parent");
    }
    stdfs::write(
        &cache_file,
        "fetched_at=1700000000\nnon_weekly_label=5h\nnon_weekly_remaining=94\nweekly_remaining=88\nweekly_reset_epoch=1700600000\n",
    )
    .expect("write cache");

    let options = rate_limits::RateLimitsOptions {
        cached: true,
        secret: Some("alpha.json".to_string()),
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 0);
}

#[test]
fn rate_limits_single_json_missing_access_token_returns_2() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-single-json-missing-token");

    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    write_secret(&secrets.join("alpha.json"), false);
    let secrets = stdfs::canonicalize(&secrets).expect("canonical secrets");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _default_all = EnvGuard::set("GEMINI_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false");

    let options = rate_limits::RateLimitsOptions {
        json: true,
        secret: Some("alpha.json".to_string()),
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 2);
}
