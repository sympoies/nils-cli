use gemini_cli::paths;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "nils-gemini-cli-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp test dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        set_env_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        remove_env_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            set_env_var(self.key, previous);
        } else {
            remove_env_var(self.key);
        }
    }
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn set_env_var(key: &str, value: impl AsRef<OsStr>) {
    // SAFETY: tests serialize env access with env_lock.
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_env_var(key: &str) {
    // SAFETY: tests serialize env access with env_lock.
    unsafe {
        std::env::remove_var(key);
    }
}

#[test]
fn paths_resolve_zdotdir_variants() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("paths-zdotdir");

    let zdotdir_env = dir.path().join("zdot_env");
    fs::create_dir_all(&zdotdir_env).expect("zdot_env");

    {
        let _zdotdir = EnvVarGuard::set("ZDOTDIR", zdotdir_env.as_os_str());
        let _preload = EnvVarGuard::remove("_ZSH_BOOTSTRAP_PRELOAD_PATH");
        assert_eq!(paths::resolve_zdotdir().expect("zdotdir"), zdotdir_env);
    }

    {
        let _zdotdir = EnvVarGuard::remove("ZDOTDIR");

        let preload = dir.path().join("a").join("b").join("preload.zsh");
        fs::create_dir_all(preload.parent().expect("parent")).expect("preload parent");

        let _preload = EnvVarGuard::set("_ZSH_BOOTSTRAP_PRELOAD_PATH", preload.as_os_str());

        let expected = dir.path().join("a");
        assert_eq!(paths::resolve_zdotdir().expect("zdotdir"), expected);
    }

    {
        let _zdotdir = EnvVarGuard::remove("ZDOTDIR");
        let _preload = EnvVarGuard::remove("_ZSH_BOOTSTRAP_PRELOAD_PATH");

        let home = dir.path().join("home");
        fs::create_dir_all(&home).expect("home");
        let _home = EnvVarGuard::set("HOME", home.as_os_str());

        assert_eq!(
            paths::resolve_zdotdir().expect("zdotdir"),
            home.join(".config").join("zsh")
        );
    }
}

#[test]
fn paths_resolve_secret_dir_from_feature_dir() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("paths-secret-dir");

    let override_dir = dir.path().join("override_secrets");
    fs::create_dir_all(&override_dir).expect("override dir");

    {
        let _secret = EnvVarGuard::set("GEMINI_SECRET_DIR", override_dir.as_os_str());
        assert_eq!(
            paths::resolve_secret_dir().expect("secret dir"),
            override_dir
        );
    }

    let script_dir = dir.path().join("scripts");
    fs::create_dir_all(&script_dir).expect("scripts dir");
    let feature_dir = script_dir.join("_features").join("gemini");
    fs::create_dir_all(&feature_dir).expect("feature dir");

    {
        let home = dir.path().join("home");
        fs::create_dir_all(&home).expect("home");
        let _secret = EnvVarGuard::remove("GEMINI_SECRET_DIR");
        let _home = EnvVarGuard::set("HOME", home.as_os_str());
        assert_eq!(
            paths::resolve_secret_dir().expect("secret dir"),
            home.join(".gemini").join("secrets")
        );
    }

    {
        let _secret = EnvVarGuard::remove("GEMINI_SECRET_DIR");
        let _home = EnvVarGuard::remove("HOME");
        let _script_dir = EnvVarGuard::set("ZSH_SCRIPT_DIR", script_dir.as_os_str());

        fs::write(feature_dir.join("init.zsh"), "#").expect("init.zsh");
        assert_eq!(
            paths::resolve_secret_dir().expect("secret dir"),
            feature_dir.join("secrets")
        );

        fs::remove_file(feature_dir.join("init.zsh")).expect("remove init.zsh");
        assert_eq!(
            paths::resolve_secret_dir().expect("secret dir"),
            feature_dir
        );
    }
}

#[test]
fn paths_resolve_secret_cache_dir_variants() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("paths-cache-dir");

    let override_dir = dir.path().join("cache_override");
    {
        let _override = EnvVarGuard::set("GEMINI_SECRET_CACHE_DIR", override_dir.as_os_str());
        assert_eq!(
            paths::resolve_secret_cache_dir().expect("secret cache dir"),
            override_dir
        );
    }

    {
        let _override = EnvVarGuard::remove("GEMINI_SECRET_CACHE_DIR");
        let cache_root = dir.path().join("cache_root");
        let _cache_root = EnvVarGuard::set("ZSH_CACHE_DIR", cache_root.as_os_str());

        assert_eq!(
            paths::resolve_secret_cache_dir().expect("secret cache dir"),
            cache_root.join("gemini").join("secrets")
        );
    }

    {
        let _override = EnvVarGuard::remove("GEMINI_SECRET_CACHE_DIR");
        let _cache_root = EnvVarGuard::remove("ZSH_CACHE_DIR");
        let home = dir.path().join("home");
        fs::create_dir_all(&home).expect("home");
        let _home = EnvVarGuard::set("HOME", home.as_os_str());

        assert_eq!(
            paths::resolve_secret_cache_dir().expect("secret cache dir"),
            home.join(".gemini").join("cache").join("secrets")
        );
    }

    {
        let _override = EnvVarGuard::remove("GEMINI_SECRET_CACHE_DIR");
        let _cache_root = EnvVarGuard::remove("ZSH_CACHE_DIR");
        let _home = EnvVarGuard::remove("HOME");
        let zdotdir = dir.path().join("zdotdir");
        let _zdotdir = EnvVarGuard::set("ZDOTDIR", zdotdir.as_os_str());
        assert_eq!(
            paths::resolve_secret_cache_dir().expect("secret cache dir"),
            zdotdir.join("cache").join("gemini").join("secrets")
        );
    }
}

#[test]
fn paths_resolve_auth_file_prefers_env() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("paths-auth-file");

    let auth_file = dir.path().join("auth.json");
    let _auth = EnvVarGuard::set("GEMINI_AUTH_FILE", auth_file.as_os_str());

    assert_eq!(paths::resolve_auth_file().expect("auth file"), auth_file);
}

#[test]
fn paths_resolve_script_dir_ignores_empty_env_override() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("paths-script-dir");

    let zdotdir = dir.path().join("zdotdir");
    fs::create_dir_all(&zdotdir).expect("zdotdir");

    let _zdotdir = EnvVarGuard::set("ZDOTDIR", zdotdir.as_os_str());
    let _script = EnvVarGuard::set("ZSH_SCRIPT_DIR", "");

    assert_eq!(
        paths::resolve_script_dir().expect("script dir"),
        zdotdir.join("scripts")
    );
}
