#![allow(dead_code, unused_imports)]
#[path = "../src/auth/mod.rs"]
mod auth;

use std::fs;
use std::path::PathBuf;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    auth::test_env_lock()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        path.push(format!("{prefix}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    fn join(&self, child: &str) -> PathBuf {
        self.path.join(child)
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
    fn set(key: &str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var_os(key);
        // SAFETY: test-scoped env mutation.
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
            // SAFETY: test-scoped env restore.
            unsafe { std::env::set_var(&self.key, value) };
        } else {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::remove_var(&self.key) };
        }
    }
}

#[test]
fn auth_auto_refresh_invalid_min_days() {
    let _lock = env_lock();

    let dir = TempDir::new("gemini-auto-refresh-invalid");
    let auth_file = dir.join("auth.json");
    fs::write(&auth_file, r#"{"last_refresh":"2025-01-20T12:34:56Z"}"#).expect("write auth");

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _min = EnvGuard::set("GEMINI_AUTO_REFRESH_MIN_DAYS", "oops");

    let code = auth::auto_refresh::run();
    assert_eq!(code, 64);
}

#[test]
fn auth_auto_refresh_unconfigured_exits_zero() {
    let _lock = env_lock();

    let dir = TempDir::new("gemini-auto-refresh-unconfigured");
    let auth_file = dir.join("missing_auth.json");
    let secrets = dir.join("secrets");
    fs::create_dir_all(&secrets).expect("secrets");

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);

    let code = auth::auto_refresh::run();
    assert_eq!(code, 0);
}

#[test]
fn auth_auto_refresh_backfills_timestamp() {
    let _lock = env_lock();

    let dir = TempDir::new("gemini-auto-refresh-backfill");
    let auth_file = dir.join("auth.json");
    let cache = dir.join("cache");
    let secrets = dir.join("secrets");
    fs::create_dir_all(&cache).expect("cache");
    fs::create_dir_all(&secrets).expect("secrets");
    let last_refresh = "2025-01-20T12:34:56Z";
    fs::write(
        &auth_file,
        format!(r#"{{"last_refresh":"{}"}}"#, last_refresh),
    )
    .expect("write auth");

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _cache = EnvGuard::set("GEMINI_SECRET_CACHE_DIR", &cache);
    let _secret = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _min = EnvGuard::set("GEMINI_AUTO_REFRESH_MIN_DAYS", "9999");

    let code = auth::auto_refresh::run();
    assert_eq!(code, 0);

    let timestamp = cache.join("auth.json.timestamp");
    assert_eq!(
        fs::read_to_string(&timestamp).expect("timestamp"),
        last_refresh
    );
}
