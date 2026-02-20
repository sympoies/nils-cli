#![allow(dead_code, unused_imports)]
#[path = "../src/auth/mod.rs"]
mod auth;

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            auth::now_epoch_seconds()
        );
        path.push(unique);
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct EnvGuard {
    key: String,
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &str, value: &str) -> Self {
        let old = std::env::var_os(key);
        // SAFETY: scoped test env mutation.
        unsafe { std::env::set_var(key, value) };
        Self {
            key: key.to_string(),
            old,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.old.take() {
            // SAFETY: scoped test env restore.
            unsafe { std::env::set_var(&self.key, value) };
        } else {
            // SAFETY: scoped test env restore.
            unsafe { std::env::remove_var(&self.key) };
        }
    }
}

#[cfg(unix)]
fn write_exe(path: &std::path::Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, content).expect("write exe");
    let mut perms = fs::metadata(path).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

#[cfg(not(unix))]
fn write_exe(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("write exe");
}

fn gemini_stub_script() -> &'static str {
    r#"#!/bin/sh
set -eu
if [ -n "${NILS_TEST_STUB_LOG:-}" ]; then
  echo "$*" >> "$NILS_TEST_STUB_LOG"
fi
exit "${GEMINI_STUB_EXIT_CODE:-0}"
"#
}

#[test]
fn auth_login_rejects_conflicting_flags() {
    let _lock = env_lock();
    let code = auth::login::run_with_json(true, true, false);
    assert_eq!(code, 64);
}

#[test]
fn auth_login_default_uses_browser_flow() {
    let _lock = env_lock();

    let dir = TempDir::new("gemini-auth-login-default");
    let stub_path = dir.path().join("gemini");
    let log_path = dir.path().join("log.txt");
    write_exe(&stub_path, gemini_stub_script());

    let path = dir.path().display().to_string();
    let _path = EnvGuard::set("PATH", &path);
    let _log = EnvGuard::set("NILS_TEST_STUB_LOG", &log_path.display().to_string());

    let code = auth::login::run_with_json(false, false, false);
    assert_eq!(code, 0);

    let log_content = fs::read_to_string(log_path).expect("read log");
    assert!(log_content.contains("login"));
}

#[test]
fn auth_login_device_code_and_api_key_map_to_expected_args() {
    let _lock = env_lock();

    let dir = TempDir::new("gemini-auth-login-flags");
    let stub_path = dir.path().join("gemini");
    let log_path = dir.path().join("log.txt");
    write_exe(&stub_path, gemini_stub_script());

    let path = dir.path().display().to_string();
    let _path = EnvGuard::set("PATH", &path);
    let _log = EnvGuard::set("NILS_TEST_STUB_LOG", &log_path.display().to_string());

    assert_eq!(auth::login::run_with_json(false, true, false), 0);
    assert_eq!(auth::login::run_with_json(true, false, false), 0);

    let log_content = fs::read_to_string(log_path).expect("read log");
    assert!(log_content.contains("login --device-auth"));
    assert!(log_content.contains("login --with-api-key"));
}
