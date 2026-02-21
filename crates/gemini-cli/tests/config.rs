#![allow(dead_code, unused_imports)]
#[path = "../src/config.rs"]
mod config;

use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, OnceLock};

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

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: tests serialize env mutations via env_lock.
        unsafe { std::env::remove_var(key) };
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

#[test]
fn config_show_with_io_prints_effective_values() {
    let _lock = env_lock();
    let _model = EnvGuard::set("GEMINI_CLI_MODEL", "m1");
    let _reasoning = EnvGuard::set("GEMINI_CLI_REASONING", "low");
    let _dangerous = EnvGuard::set("GEMINI_ALLOW_DANGEROUS_ENABLED", "true");
    let _secret = EnvGuard::set("GEMINI_SECRET_DIR", "/tmp/secrets");
    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", "/tmp/auth.json");
    let _cache = EnvGuard::set("GEMINI_SECRET_CACHE_DIR", "/tmp/cache/secrets");
    let _starship = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let _auto_refresh = EnvGuard::set("GEMINI_AUTO_REFRESH_ENABLED", "true");
    let _min_days = EnvGuard::set("GEMINI_AUTO_REFRESH_MIN_DAYS", "9");

    let mut out: Vec<u8> = Vec::new();
    assert_eq!(config::show_with_io(&mut out), 0);
    let output = String::from_utf8_lossy(&out);
    assert!(output.contains("GEMINI_CLI_MODEL=m1\n"));
    assert!(output.contains("GEMINI_CLI_REASONING=low\n"));
    assert!(output.contains("GEMINI_ALLOW_DANGEROUS_ENABLED=true\n"));
    assert!(output.contains("GEMINI_SECRET_DIR=/tmp/secrets\n"));
    assert!(output.contains("GEMINI_AUTH_FILE=/tmp/auth.json\n"));
    assert!(output.contains("GEMINI_SECRET_CACHE_DIR=/tmp/cache/secrets\n"));
    assert!(output.contains("GEMINI_STARSHIP_ENABLED=true\n"));
    assert!(output.contains("GEMINI_AUTO_REFRESH_ENABLED=true\n"));
    assert!(output.contains("GEMINI_AUTO_REFRESH_MIN_DAYS=9\n"));
}

#[test]
fn config_show_with_io_prints_blank_paths_when_unresolvable() {
    let _lock = env_lock();
    let _home = EnvGuard::remove("HOME");
    let _zdotdir = EnvGuard::remove("ZDOTDIR");
    let _script = EnvGuard::remove("ZSH_SCRIPT_DIR");
    let _preload = EnvGuard::remove("_ZSH_BOOTSTRAP_PRELOAD_PATH");
    let _cache_root = EnvGuard::remove("ZSH_CACHE_DIR");
    let _secret = EnvGuard::remove("GEMINI_SECRET_DIR");
    let _auth = EnvGuard::remove("GEMINI_AUTH_FILE");
    let _secret_cache = EnvGuard::remove("GEMINI_SECRET_CACHE_DIR");

    let mut out: Vec<u8> = Vec::new();
    assert_eq!(config::show_with_io(&mut out), 0);
    let output = String::from_utf8_lossy(&out);
    assert!(output.contains("GEMINI_SECRET_DIR=\n"));
    assert!(output.contains("GEMINI_AUTH_FILE=\n"));
    assert!(output.contains("GEMINI_SECRET_CACHE_DIR=\n"));
}

#[test]
fn config_set_with_io_model_quotes_value() {
    let _lock = env_lock();
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();

    assert_eq!(
        config::set_with_io("model", "gemini-test", &mut out, &mut err),
        0
    );
    assert_eq!(
        String::from_utf8_lossy(&out),
        "export GEMINI_CLI_MODEL='gemini-test'\n"
    );
    assert!(err.is_empty());
}

#[test]
fn config_set_with_io_reason_alias_is_supported() {
    let _lock = env_lock();
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();

    assert_eq!(config::set_with_io("reason", "high", &mut out, &mut err), 0);
    assert_eq!(
        String::from_utf8_lossy(&out),
        "export GEMINI_CLI_REASONING='high'\n"
    );
    assert!(err.is_empty());
}

#[test]
fn config_set_with_io_dangerous_rejects_invalid_values() {
    let _lock = env_lock();
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();

    assert_eq!(
        config::set_with_io("dangerous", "maybe", &mut out, &mut err),
        64
    );
    assert!(out.is_empty());
    assert!(
        String::from_utf8_lossy(&err).contains("dangerous must be true|false"),
        "stderr was: {}",
        String::from_utf8_lossy(&err)
    );
}

#[test]
fn config_set_with_io_unknown_key_returns_64() {
    let _lock = env_lock();
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();

    assert_eq!(config::set_with_io("wat", "x", &mut out, &mut err), 64);
    assert!(out.is_empty());
    let stderr = String::from_utf8_lossy(&err);
    assert!(stderr.contains("unknown key"));
    assert!(stderr.contains("model|reasoning|dangerous"));
}

#[test]
fn config_set_with_io_escapes_single_quotes() {
    let _lock = env_lock();
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();

    assert_eq!(config::set_with_io("model", "a'b", &mut out, &mut err), 0);
    assert_eq!(
        String::from_utf8_lossy(&out),
        "export GEMINI_CLI_MODEL='a'\"'\"'b'\n"
    );
    assert!(err.is_empty());
}
