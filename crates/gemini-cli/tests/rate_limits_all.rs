#![allow(dead_code, unused_imports)]
#[path = "../src/auth/mod.rs"]
mod auth;
#[path = "../src/fs.rs"]
mod fs;
#[path = "../src/json.rs"]
mod json;
#[path = "../src/paths.rs"]
mod paths;
#[path = "../src/rate_limits/mod.rs"]
mod rate_limits;

use std::ffi::{OsStr, OsString};
use std::fs as stdfs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock")
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

fn write_secret(path: PathBuf) {
    stdfs::write(
        path,
        r#"{"tokens":{"access_token":"tok","account_id":"acct_001"}}"#,
    )
    .expect("write secret");
}

#[test]
fn rate_limits_all_missing_secret_dir_returns_1() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-all-missing-secret-dir");
    let missing = dir.join("missing");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &missing);
    let options = rate_limits::RateLimitsOptions {
        all: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 1);
}

#[test]
fn rate_limits_all_with_positional_secret_returns_64() {
    let _lock = env_lock();
    let options = rate_limits::RateLimitsOptions {
        all: true,
        secret: Some("alpha.json".to_string()),
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 64);
}

#[test]
fn rate_limits_all_json_empty_secret_dir_returns_1() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-all-json-empty-secret-dir");
    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let options = rate_limits::RateLimitsOptions {
        all: true,
        json: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 1);
}

#[test]
fn rate_limits_all_cached_success_returns_0() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-all-cached-success");

    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    let alpha = secrets.join("alpha.json");
    let beta = secrets.join("beta.json");
    write_secret(alpha.clone());
    write_secret(beta.clone());

    let cache_root = dir.join("cache-root");
    stdfs::create_dir_all(&cache_root).expect("cache root");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);

    for target in [&alpha, &beta] {
        let cache = rate_limits::cache_file_for_target(target).expect("cache file");
        if let Some(parent) = cache.parent() {
            stdfs::create_dir_all(parent).expect("cache parent");
        }
        stdfs::write(
            cache,
            "fetched_at=1700000000\nnon_weekly_label=5h\nnon_weekly_remaining=80\nweekly_remaining=70\nweekly_reset_epoch=1700600000\n",
        )
        .expect("write cache");
    }

    let options = rate_limits::RateLimitsOptions {
        all: true,
        cached: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 0);
}

#[test]
fn rate_limits_all_cached_partial_failure_returns_1() {
    let _lock = env_lock();
    let dir = TestDir::new("rate-limits-all-cached-partial-failure");

    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    let alpha = secrets.join("alpha.json");
    let beta = secrets.join("beta.json");
    write_secret(alpha.clone());
    write_secret(beta.clone());

    let cache_root = dir.join("cache-root");
    stdfs::create_dir_all(&cache_root).expect("cache root");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);

    let alpha_cache = rate_limits::cache_file_for_target(&alpha).expect("alpha cache");
    if let Some(parent) = alpha_cache.parent() {
        stdfs::create_dir_all(parent).expect("cache parent");
    }
    stdfs::write(
        alpha_cache,
        "fetched_at=1700000000\nnon_weekly_label=5h\nnon_weekly_remaining=80\nweekly_remaining=70\nweekly_reset_epoch=1700600000\n",
    )
    .expect("write alpha cache");

    let options = rate_limits::RateLimitsOptions {
        all: true,
        cached: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 1);
}
