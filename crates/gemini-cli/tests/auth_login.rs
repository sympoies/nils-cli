use gemini_cli::auth;

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    match LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
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
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0)
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

fn curl_stub_script() -> &'static str {
    r#"#!/bin/sh
set -eu
case "$*" in
  *openidconnect.googleapis.com/v1/userinfo*)
    cat <<'EOF'
{"email":"alpha@example.com"}
__HTTP_STATUS__:200
EOF
    exit 0
    ;;
esac
cat <<'EOF'
{"error":"unexpected"}
__HTTP_STATUS__:400
EOF
exit 0
"#
}

fn gemini_stub_script() -> &'static str {
    r#"#!/bin/sh
set -eu
cat > "$GEMINI_AUTH_FILE" <<'EOF'
{"access_token":"tok","id_token":"header.payload.sig"}
EOF
exit 0
"#
}

#[test]
fn auth_login_rejects_conflicting_flags() {
    let _lock = env_lock();
    let code = auth::login::run_with_json(true, true, false);
    assert_eq!(code, 64);
}

#[test]
fn auth_login_api_key_requires_env() {
    let _lock = env_lock();
    let _api = EnvGuard::set("GEMINI_API_KEY", "");
    let _google = EnvGuard::set("GOOGLE_API_KEY", "");
    let code = auth::login::run_with_json(true, false, false);
    assert_eq!(code, 64);
}

#[test]
fn auth_login_api_key_succeeds_with_env() {
    let _lock = env_lock();
    let _api = EnvGuard::set("GEMINI_API_KEY", "dummy-key");
    let code = auth::login::run_with_json(true, false, false);
    assert_eq!(code, 0);
}

#[test]
fn auth_login_browser_and_device_code_use_userinfo_flow() {
    let _lock = env_lock();
    let dir = TempDir::new("gemini-auth-login-browser");
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    let curl_path = bin_dir.join("curl");
    let gemini_path = bin_dir.join("gemini");
    write_exe(&curl_path, curl_stub_script());
    write_exe(&gemini_path, gemini_stub_script());

    let auth_file = dir.path().join("oauth_creds.json");
    fs::write(
        &auth_file,
        r#"{"access_token":"tok","id_token":"header.payload.sig"}"#,
    )
    .expect("write auth");

    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _path = EnvGuard::set("PATH", &path);
    let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());

    assert_eq!(auth::login::run_with_json(false, false, false), 0);
    assert_eq!(auth::login::run_with_json(false, true, false), 0);
}
