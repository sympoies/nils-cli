use gemini_cli::{rate_limits, starship};

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

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

fn write_auth_secret(dir: &TestDir) -> (PathBuf, PathBuf, PathBuf) {
    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    let auth_file = secrets.join("alpha.json");
    stdfs::write(
        &auth_file,
        r#"{"tokens":{"access_token":"tok","account_id":"acct_001"}}"#,
    )
    .expect("write auth");

    let cache_root = dir.join("cache-root");
    stdfs::create_dir_all(&cache_root).expect("cache root");
    (auth_file, secrets, cache_root)
}

fn write_auth_with_id_token(path: &Path, id_token: &str) {
    stdfs::write(
        path,
        format!(
            "{{\"tokens\":{{\"id_token\":\"{id_token}\",\"access_token\":\"tok\",\"account_id\":\"acct_001\"}}}}"
        ),
    )
    .expect("write auth with id token");
}

fn set_fast_fail_refresh_env() -> (EnvGuard, EnvGuard, EnvGuard) {
    (
        EnvGuard::set("CODE_ASSIST_ENDPOINT", "http://127.0.0.1:9/"),
        EnvGuard::set("GEMINI_STARSHIP_CURL_CONNECT_TIMEOUT_SECONDS", "1"),
        EnvGuard::set("GEMINI_STARSHIP_CURL_MAX_TIME_SECONDS", "1"),
    )
}

#[test]
fn starship_disabled_returns_0() {
    let _lock = env_lock();
    let options = starship::StarshipOptions::default();
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "false");
    assert_eq!(starship::run(&options), 0);
}

#[test]
fn starship_is_enabled_flag_reflects_env() {
    let _lock = env_lock();
    let options = starship::StarshipOptions {
        is_enabled: true,
        ..Default::default()
    };

    let _enabled_false = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "false");
    assert_eq!(starship::run(&options), 1);
    drop(_enabled_false);

    let _enabled_true = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    assert_eq!(starship::run(&options), 0);
}

#[test]
fn starship_invalid_ttl_returns_2() {
    let _lock = env_lock();
    let options = starship::StarshipOptions {
        ttl: Some("bogus".to_string()),
        ..Default::default()
    };
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    assert_eq!(starship::run(&options), 2);
}

#[test]
fn starship_non_stale_cache_skips_failed_refresh() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-non-stale-cache");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let _base = EnvGuard::set("CODE_ASSIST_ENDPOINT", "http://127.0.0.1:9/");
    let _connect = EnvGuard::set("GEMINI_STARSHIP_CURL_CONNECT_TIMEOUT_SECONDS", "1");
    let _max_time = EnvGuard::set("GEMINI_STARSHIP_CURL_MAX_TIME_SECONDS", "1");

    rate_limits::write_starship_cache(
        &auth_file,
        now_epoch().max(1),
        "5h",
        94,
        88,
        1700600000,
        Some(1700003600),
    )
    .expect("write cache");
    let cache_file = rate_limits::cache_file_for_target(&auth_file).expect("cache file");
    let before = stdfs::read_to_string(&cache_file).expect("cache before");

    let options = starship::StarshipOptions {
        time_format: Some("%Y-%m-%dT%H:%MZ".to_string()),
        ..Default::default()
    };
    assert_eq!(starship::run(&options), 0);

    let after = stdfs::read_to_string(&cache_file).expect("cache after");
    assert_eq!(before, after);
}

#[test]
fn starship_stale_cache_with_failed_refresh_returns_0() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-stale-cache-failed-refresh");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let _base = EnvGuard::set("CODE_ASSIST_ENDPOINT", "http://127.0.0.1:9/");
    let _connect = EnvGuard::set("GEMINI_STARSHIP_CURL_CONNECT_TIMEOUT_SECONDS", "1");
    let _max_time = EnvGuard::set("GEMINI_STARSHIP_CURL_MAX_TIME_SECONDS", "1");

    rate_limits::write_starship_cache(
        &auth_file,
        now_epoch().saturating_sub(10).max(1),
        "5h",
        1,
        2,
        1700600000,
        Some(1700003600),
    )
    .expect("write cache");

    let options = starship::StarshipOptions {
        ttl: Some("1s".to_string()),
        time_format: Some("%Y-%m-%dT%H:%MZ".to_string()),
        ..Default::default()
    };
    assert_eq!(starship::run(&options), 0);
}

#[test]
fn starship_valid_ttl_units_parse_when_disabled() {
    let _lock = env_lock();
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "false");

    for ttl in ["1s", "2m", "3h", "4d", "1w", "5"] {
        let options = starship::StarshipOptions {
            ttl: Some(ttl.to_string()),
            ..Default::default()
        };
        assert_eq!(starship::run(&options), 0, "ttl={ttl}");
    }
}

#[test]
fn starship_zero_ttl_returns_2() {
    let _lock = env_lock();
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "false");
    let options = starship::StarshipOptions {
        ttl: Some("0".to_string()),
        ..Default::default()
    };
    assert_eq!(starship::run(&options), 2);
}

#[test]
fn starship_env_ttl_parse_when_cli_absent() {
    let _lock = env_lock();
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "false");
    let _ttl = EnvGuard::set("GEMINI_STARSHIP_TTL", "2m");
    let options = starship::StarshipOptions::default();
    assert_eq!(starship::run(&options), 0);
}

#[test]
fn starship_missing_auth_file_returns_0() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-missing-auth-file");
    let missing = dir.join("missing.json");

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &missing);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}

#[test]
fn starship_unresolvable_auth_path_returns_0() {
    let _lock = env_lock();
    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", "");
    let _home = EnvGuard::set("HOME", "");
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}

#[test]
fn starship_missing_cache_root_is_treated_as_no_cache() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-missing-cache-root");
    let (auth_file, secrets, _cache_root) = write_auth_secret(&dir);

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _home = EnvGuard::set("HOME", "");
    let _zdot = EnvGuard::set("ZDOTDIR", "");
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", "");
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let (_base, _connect, _max_time) = set_fast_fail_refresh_env();

    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}

#[test]
fn starship_invalid_cache_entry_is_ignored() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-invalid-cache-entry");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let (_base, _connect, _max_time) = set_fast_fail_refresh_env();

    let cache_file = rate_limits::cache_file_for_target(&auth_file).expect("cache file");
    if let Some(parent) = cache_file.parent() {
        stdfs::create_dir_all(parent).expect("cache parent");
    }
    stdfs::write(&cache_file, "not-a-valid-cache").expect("write invalid cache");

    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}

#[test]
fn starship_non_positive_fetched_at_is_treated_as_stale() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-non-positive-fetched-at");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let (_base, _connect, _max_time) = set_fast_fail_refresh_env();

    rate_limits::write_starship_cache(&auth_file, 0, "5h", 94, 88, 1700600000, Some(1700003600))
        .expect("write cache");

    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}

#[test]
fn starship_default_time_format_paths_execute() {
    let _lock = env_lock();
    let dir = TestDir::new("starship-default-time-format");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let (_base, _connect, _max_time) = set_fast_fail_refresh_env();

    rate_limits::write_starship_cache(
        &auth_file,
        now_epoch().max(1),
        "5h",
        94,
        88,
        1700600000,
        Some(1700003600),
    )
    .expect("write cache");

    let with_timezone = starship::StarshipOptions {
        show_timezone: true,
        ..Default::default()
    };
    assert_eq!(starship::run(&with_timezone), 0);

    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}

#[test]
fn starship_email_name_source_paths_execute() {
    const TOKEN_WITH_EMAIL: &str =
        "x.eyJlbWFpbCI6ImFsaWNlQGV4YW1wbGUuY29tIiwic3ViIjoiYWxpY2UtaWQifQ.y";
    const TOKEN_WITH_SUB_ONLY: &str = "x.eyJzdWIiOiJhbGljZS1pZCJ9.y";

    let _lock = env_lock();
    let dir = TestDir::new("starship-email-name-source");
    let (auth_file, secrets, cache_root) = write_auth_secret(&dir);

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let _source = EnvGuard::set("GEMINI_STARSHIP_NAME_SOURCE", "email");
    let (_base, _connect, _max_time) = set_fast_fail_refresh_env();

    write_auth_with_id_token(&auth_file, TOKEN_WITH_EMAIL);
    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);

    write_auth_with_id_token(&auth_file, TOKEN_WITH_SUB_ONLY);
    let _fallback = EnvGuard::set("GEMINI_STARSHIP_SHOW_FALLBACK_NAME_ENABLED", "true");
    let _show_full = EnvGuard::set("GEMINI_STARSHIP_SHOW_FULL_EMAIL_ENABLED", "true");
    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
    drop(_show_full);
    drop(_fallback);

    let _fallback_off = EnvGuard::set("GEMINI_STARSHIP_SHOW_FALLBACK_NAME_ENABLED", "false");
    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}

#[test]
fn starship_secret_name_fallback_paths_execute() {
    const TOKEN_WITH_SUB_ONLY: &str = "x.eyJzdWIiOiJhbGljZS1pZCJ9.y";

    let _lock = env_lock();
    let dir = TestDir::new("starship-secret-name-fallback");
    let auth_file = dir.join("auth.json");
    write_auth_with_id_token(&auth_file, TOKEN_WITH_SUB_ONLY);

    let secrets = dir.join("secrets");
    stdfs::create_dir_all(&secrets).expect("secrets");
    let cache_root = dir.join("cache-root");
    stdfs::create_dir_all(&cache_root).expect("cache root");

    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file);
    let _secret_dir = EnvGuard::set("GEMINI_SECRET_DIR", &secrets);
    let _cache_root = EnvGuard::set("ZSH_CACHE_DIR", &cache_root);
    let _enabled = EnvGuard::set("GEMINI_STARSHIP_ENABLED", "true");
    let (_base, _connect, _max_time) = set_fast_fail_refresh_env();

    let _fallback_on = EnvGuard::set("GEMINI_STARSHIP_SHOW_FALLBACK_NAME_ENABLED", "true");
    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
    drop(_fallback_on);

    let _fallback_off = EnvGuard::set("GEMINI_STARSHIP_SHOW_FALLBACK_NAME_ENABLED", "false");
    assert_eq!(starship::run(&starship::StarshipOptions::default()), 0);
}
