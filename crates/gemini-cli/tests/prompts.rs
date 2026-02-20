#![allow(dead_code, unused_imports)]
#[path = "../src/paths.rs"]
mod paths;
#[path = "../src/prompts.rs"]
mod prompts;

use prompts::PromptTemplateError;
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

fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(mode);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

#[test]
fn prompts_resolve_uses_feature_dir_fallback_when_zdotdir_prompts_missing() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("prompts-feature-fallback");

    let zdotdir = dir.path().join("zdotdir");
    fs::create_dir_all(&zdotdir).expect("zdotdir");

    let script_dir = dir.path().join("scripts");
    let feature_prompts = script_dir.join("_features").join("gemini").join("prompts");
    fs::create_dir_all(&feature_prompts).expect("feature prompts");
    fs::write(
        feature_prompts.join("actionable-advice.md"),
        "Hello $ARGUMENTS\n",
    )
    .expect("write template");

    let _zdotdir_env = EnvVarGuard::set("ZDOTDIR", zdotdir.as_os_str());
    let _script_env = EnvVarGuard::set("ZSH_SCRIPT_DIR", script_dir.as_os_str());

    let resolved = prompts::resolve_prompts_dir().expect("resolve prompts dir");
    assert_eq!(resolved, feature_prompts);

    let (path, content) = prompts::read_template("actionable-advice").expect("read template");
    assert_eq!(path, feature_prompts.join("actionable-advice.md"));
    assert!(content.contains("Hello $ARGUMENTS"));
}

#[test]
fn prompts_resolve_returns_none_when_no_prompts_dirs_exist() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("prompts-no-dirs");

    let zdotdir = dir.path().join("zdotdir");
    fs::create_dir_all(&zdotdir).expect("zdotdir");

    let script_dir = dir.path().join("scripts");
    let feature_dir = script_dir.join("_features").join("gemini");
    fs::create_dir_all(&feature_dir).expect("feature dir");

    let _zdotdir_env = EnvVarGuard::set("ZDOTDIR", zdotdir.as_os_str());
    let _script_env = EnvVarGuard::set("ZSH_SCRIPT_DIR", script_dir.as_os_str());

    assert!(prompts::resolve_prompts_dir().is_none());
}

#[test]
fn prompts_read_template_errors_when_prompts_dir_missing() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("prompts-missing-dir");

    let zdotdir = dir.path().join("zdotdir");
    fs::create_dir_all(&zdotdir).expect("zdotdir");
    let script_dir = dir.path().join("scripts");
    fs::create_dir_all(&script_dir).expect("scripts dir");

    let _zdotdir_env = EnvVarGuard::set("ZDOTDIR", zdotdir.as_os_str());
    let _script_env = EnvVarGuard::set("ZSH_SCRIPT_DIR", script_dir.as_os_str());

    let err = prompts::read_template("anything").expect_err("missing prompts dir");
    assert!(matches!(err, PromptTemplateError::PromptsDirNotFound));
}

#[test]
fn prompts_read_template_errors_when_template_missing() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("prompts-missing-template");

    let zdotdir = dir.path().join("zdotdir");
    let prompts_dir = zdotdir.join("prompts");
    fs::create_dir_all(&prompts_dir).expect("prompts dir");

    let _zdotdir_env = EnvVarGuard::set("ZDOTDIR", zdotdir.as_os_str());

    let err = prompts::read_template("missing-template").expect_err("missing template");
    match err {
        PromptTemplateError::TemplateMissing { path } => {
            assert!(path.ends_with("missing-template.md"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn prompts_read_template_errors_when_template_unreadable() {
    let _lock = env_lock().lock().expect("lock");
    let dir = TestDir::new("prompts-unreadable");

    let zdotdir = dir.path().join("zdotdir");
    let prompts_dir = zdotdir.join("prompts");
    fs::create_dir_all(&prompts_dir).expect("prompts dir");
    let template = prompts_dir.join("unreadable.md");
    fs::write(&template, "hi").expect("write template");
    set_mode(&template, 0o000);

    let _zdotdir_env = EnvVarGuard::set("ZDOTDIR", zdotdir.as_os_str());

    #[cfg(unix)]
    {
        let err = prompts::read_template("unreadable").expect_err("unreadable template");
        match err {
            PromptTemplateError::ReadFailed { path } => assert_eq!(path, template),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[cfg(not(unix))]
    {
        let (path, content) = prompts::read_template("unreadable").expect("read template");
        assert_eq!(path, template);
        assert_eq!(content, "hi");
    }
}
