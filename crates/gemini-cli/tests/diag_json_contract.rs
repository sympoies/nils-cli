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

#[test]
fn diag_json_contract_schema_constants_are_stable() {
    assert_eq!(
        rate_limits::DIAG_SCHEMA_VERSION,
        "gemini-cli.diag.rate-limits.v1"
    );
    assert_eq!(rate_limits::DIAG_COMMAND, "diag rate-limits");
}

#[test]
fn diag_json_contract_single_missing_access_token_returns_2() {
    let _lock = env_lock();
    let dir = TestDir::new("diag-json-contract-missing-token");

    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    stdfs::write(
        secrets.join("alpha.json"),
        r#"{"tokens":{"account_id":"acct_001"}}"#,
    )
    .expect("write secret");

    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _default_all = EnvGuard::set("GEMINI_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false");

    let options = rate_limits::RateLimitsOptions {
        json: true,
        secret: Some("alpha.json".to_string()),
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 2);
}

#[test]
fn diag_json_contract_all_empty_secret_dir_returns_1() {
    let _lock = env_lock();
    let dir = TestDir::new("diag-json-contract-all-empty");
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
fn diag_json_contract_rejects_one_line_with_json() {
    let _lock = env_lock();
    let options = rate_limits::RateLimitsOptions {
        json: true,
        one_line: true,
        ..Default::default()
    };
    assert_eq!(rate_limits::run(&options), 64);
}
