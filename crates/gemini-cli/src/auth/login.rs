use std::io::IsTerminal;
use std::path::Path;
use std::process::Command;
use std::{fs, io};

use crate::auth::output;

const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LoginMethod {
    GeminiBrowser,
    GeminiDeviceCode,
    ApiKey,
}

pub fn run(api_key: bool, device_code: bool) -> i32 {
    run_with_json(api_key, device_code, false)
}

pub fn run_with_json(api_key: bool, device_code: bool, output_json: bool) -> i32 {
    let method = match resolve_method(api_key, device_code) {
        Ok(method) => method,
        Err((code, message, details)) => {
            if output_json {
                let _ = output::emit_error("auth login", "invalid-usage", message, details);
            } else {
                eprintln!("{message}");
            }
            return code;
        }
    };

    if method == LoginMethod::ApiKey {
        return run_api_key_login(output_json);
    }

    run_oauth_login(method, output_json)
}

fn run_api_key_login(output_json: bool) -> i32 {
    let source = if std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some()
    {
        Some("GEMINI_API_KEY")
    } else if std::env::var("GOOGLE_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some()
    {
        Some("GOOGLE_API_KEY")
    } else {
        None
    };

    let Some(source) = source else {
        if output_json {
            let _ = output::emit_error(
                "auth login",
                "missing-api-key",
                "gemini-login: set GEMINI_API_KEY or GOOGLE_API_KEY before using --api-key",
                None,
            );
        } else {
            eprintln!("gemini-login: set GEMINI_API_KEY or GOOGLE_API_KEY before using --api-key");
        }
        return 64;
    };

    if output_json {
        let _ = output::emit_result(
            "auth login",
            output::obj(vec![
                ("method", output::s("api-key")),
                ("provider", output::s("gemini-api")),
                ("completed", output::b(true)),
                ("source", output::s(source)),
            ]),
        );
    } else {
        println!("gemini: login complete (method: api-key)");
    }

    0
}

fn run_oauth_login(method: LoginMethod, output_json: bool) -> i32 {
    if output_json {
        return run_oauth_session_check(method, true);
    }

    let interactive_terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !interactive_terminal {
        // Keep non-interactive automation stable.
        return run_oauth_session_check(method, false);
    }

    run_oauth_interactive_login(method)
}

fn run_oauth_session_check(method: LoginMethod, output_json: bool) -> i32 {
    let auth_file = match gemini_core::paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            emit_login_error(
                output_json,
                "auth-file-not-configured",
                "gemini-login: GEMINI_AUTH_FILE is not configured".to_string(),
                None,
            );
            return 1;
        }
    };

    if !auth_file.is_file() {
        emit_login_error(
            output_json,
            "auth-file-not-found",
            format!("gemini-login: auth file not found: {}", auth_file.display()),
            Some(output::obj(vec![(
                "auth_file",
                output::s(auth_file.display().to_string()),
            )])),
        );
        return 1;
    }

    let mut refresh_attempted = false;
    if has_refresh_token(&auth_file) {
        refresh_attempted = true;
        // Refresh failures are tolerated here if the current access token remains valid.
        let _ = crate::auth::refresh::run_silent(&[]);
    }

    let auth_json = match gemini_core::json::read_json(&auth_file) {
        Ok(value) => value,
        Err(err) => {
            emit_login_error(
                output_json,
                "auth-read-failed",
                format!(
                    "gemini-login: failed to read auth file {}",
                    auth_file.display()
                ),
                Some(output::obj(vec![
                    ("auth_file", output::s(auth_file.display().to_string())),
                    ("error", output::s(err.to_string())),
                ])),
            );
            return 1;
        }
    };

    let access_token = access_token_from_json(&auth_json);
    let access_token = match access_token {
        Some(token) => token,
        None => {
            emit_login_error(
                output_json,
                "missing-access-token",
                format!(
                    "gemini-login: missing access token in {}",
                    auth_file.display()
                ),
                Some(output::obj(vec![(
                    "auth_file",
                    output::s(auth_file.display().to_string()),
                )])),
            );
            return 2;
        }
    };

    let userinfo = match fetch_google_userinfo(&access_token) {
        Ok(value) => value,
        Err(err) => {
            emit_login_error(output_json, err.code, err.message, err.details);
            return err.exit_code;
        }
    };

    let email = userinfo
        .get("email")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    if output_json {
        let _ = output::emit_result(
            "auth login",
            output::obj(vec![
                ("method", output::s(method.as_str())),
                ("provider", output::s(method.provider())),
                ("completed", output::b(true)),
                ("auth_file", output::s(auth_file.display().to_string())),
                (
                    "email",
                    if email.is_empty() {
                        output::null()
                    } else {
                        output::s(email)
                    },
                ),
                ("refresh_attempted", output::b(refresh_attempted)),
            ]),
        );
    } else {
        println!("gemini: login complete (method: {})", method.as_str());
    }

    0
}

fn run_oauth_interactive_login(method: LoginMethod) -> i32 {
    let auth_file = match gemini_core::paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            emit_login_error(
                false,
                "auth-file-not-configured",
                "gemini-login: GEMINI_AUTH_FILE is not configured".to_string(),
                None,
            );
            return 1;
        }
    };

    let backup = match backup_auth_file(&auth_file) {
        Ok(backup) => backup,
        Err(err) => {
            emit_login_error(false, "auth-read-failed", err.to_string(), None);
            return 1;
        }
    };

    if let Some(parent) = auth_file.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        emit_login_error(
            false,
            "auth-dir-create-failed",
            format!(
                "gemini-login: failed to prepare auth directory {}: {err}",
                parent.display()
            ),
            Some(output::obj(vec![(
                "auth_file",
                output::s(auth_file.display().to_string()),
            )])),
        );
        return 1;
    }

    if auth_file.is_file()
        && let Err(err) = fs::remove_file(&auth_file)
    {
        emit_login_error(
            false,
            "auth-file-remove-failed",
            format!(
                "gemini-login: failed to remove auth file {}: {err}",
                auth_file.display()
            ),
            Some(output::obj(vec![(
                "auth_file",
                output::s(auth_file.display().to_string()),
            )])),
        );
        return 1;
    }

    if method == LoginMethod::GeminiBrowser {
        println!("Code Assist login required. Opening authentication page in your browser.");
    }

    let status = match run_gemini_interactive_login(method, &auth_file) {
        Ok(status) => status,
        Err(err) => {
            let _ = restore_auth_backup(&auth_file, backup.as_deref());
            emit_login_error(false, err.code, err.message, err.details);
            return err.exit_code;
        }
    };

    if !status.success() {
        let _ = restore_auth_backup(&auth_file, backup.as_deref());
        let exit_code = status.code().unwrap_or(1).max(1);
        emit_login_error(
            false,
            "login-failed",
            format!("gemini-login: login failed for method {}", method.as_str()),
            Some(output::obj(vec![
                ("method", output::s(method.as_str())),
                ("exit_code", output::n(i64::from(exit_code))),
            ])),
        );
        return exit_code;
    }

    let auth_json = match gemini_core::json::read_json(&auth_file) {
        Ok(value) => value,
        Err(err) => {
            let _ = restore_auth_backup(&auth_file, backup.as_deref());
            emit_login_error(
                false,
                "auth-read-failed",
                format!(
                    "gemini-login: login completed but failed to read auth file {}: {err}",
                    auth_file.display()
                ),
                Some(output::obj(vec![(
                    "auth_file",
                    output::s(auth_file.display().to_string()),
                )])),
            );
            return 1;
        }
    };

    let access_token = match access_token_from_json(&auth_json) {
        Some(token) => token,
        None => {
            let _ = restore_auth_backup(&auth_file, backup.as_deref());
            emit_login_error(
                false,
                "missing-access-token",
                format!(
                    "gemini-login: login completed but auth file is missing access token: {}",
                    auth_file.display()
                ),
                Some(output::obj(vec![(
                    "auth_file",
                    output::s(auth_file.display().to_string()),
                )])),
            );
            return 2;
        }
    };

    if let Err(err) = fetch_google_userinfo(&access_token) {
        let _ = restore_auth_backup(&auth_file, backup.as_deref());
        emit_login_error(false, err.code, err.message, err.details);
        return err.exit_code;
    }

    println!("gemini: login complete (method: {})", method.as_str());
    0
}

fn backup_auth_file(path: &Path) -> io::Result<Option<Vec<u8>>> {
    if !path.is_file() {
        return Ok(None);
    }
    fs::read(path).map(Some)
}

fn restore_auth_backup(path: &Path, backup: Option<&[u8]>) -> io::Result<()> {
    match backup {
        Some(contents) => crate::auth::write_atomic(path, contents, crate::auth::SECRET_FILE_MODE),
        None => {
            if path.is_file() {
                fs::remove_file(path)
            } else {
                Ok(())
            }
        }
    }
}

fn run_gemini_interactive_login(
    method: LoginMethod,
    auth_file: &Path,
) -> Result<std::process::ExitStatus, LoginError> {
    let mut command = Command::new("gemini");
    command.arg("--prompt-interactive").arg("/quit");
    if method == LoginMethod::GeminiBrowser {
        // Auto-accept the browser launch prompt without shell-side input hacks.
        command.arg("--yolo");
    }
    command.env("GEMINI_AUTH_FILE", auth_file.to_string_lossy().to_string());

    if method == LoginMethod::GeminiDeviceCode {
        command.env("NO_BROWSER", "true");
    } else {
        command.env_remove("NO_BROWSER");
    }

    let status = command.status().map_err(|_| LoginError {
        code: "login-exec-failed",
        message: format!(
            "gemini-login: failed to run `gemini` for method {}",
            method.as_str()
        ),
        details: Some(output::obj(vec![("method", output::s(method.as_str()))])),
        exit_code: 1,
    })?;

    if !auth_file.is_file() {
        return Err(LoginError {
            code: "auth-file-not-found",
            message: format!(
                "gemini-login: interactive login did not produce auth file: {}",
                auth_file.display()
            ),
            details: Some(output::obj(vec![
                ("method", output::s(method.as_str())),
                ("auth_file", output::s(auth_file.display().to_string())),
                ("exit_code", output::n(status.code().unwrap_or(0) as i64)),
            ])),
            exit_code: 1,
        });
    }

    Ok(status)
}

struct LoginError {
    code: &'static str,
    message: String,
    details: Option<output::JsonValue>,
    exit_code: i32,
}

fn fetch_google_userinfo(access_token: &str) -> Result<serde_json::Value, LoginError> {
    let connect_timeout = env_timeout("GEMINI_LOGIN_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("GEMINI_LOGIN_CURL_MAX_TIME_SECONDS", 8);

    let response = Command::new("curl")
        .arg("-sS")
        .arg("--connect-timeout")
        .arg(connect_timeout.to_string())
        .arg("--max-time")
        .arg(max_time.to_string())
        .arg("-H")
        .arg(format!("Authorization: Bearer {access_token}"))
        .arg("-H")
        .arg("Accept: application/json")
        .arg(GOOGLE_USERINFO_URL)
        .arg("-w")
        .arg("\n__HTTP_STATUS__:%{http_code}")
        .output()
        .map_err(|_| LoginError {
            code: "login-request-failed",
            message: format!("gemini-login: failed to query {GOOGLE_USERINFO_URL}"),
            details: Some(output::obj(vec![(
                "endpoint",
                output::s(GOOGLE_USERINFO_URL),
            )])),
            exit_code: 3,
        })?;

    if !response.status.success() {
        return Err(LoginError {
            code: "login-request-failed",
            message: format!("gemini-login: failed to query {GOOGLE_USERINFO_URL}"),
            details: Some(output::obj(vec![(
                "endpoint",
                output::s(GOOGLE_USERINFO_URL),
            )])),
            exit_code: 3,
        });
    }

    let response_text = String::from_utf8_lossy(&response.stdout).to_string();
    let (body, http_status) = split_http_status_marker(&response_text);
    if http_status != 200 {
        let summary = http_error_summary(&body);
        let mut details = vec![
            ("endpoint".to_string(), output::s(GOOGLE_USERINFO_URL)),
            ("http_status".to_string(), output::n(http_status as i64)),
        ];
        if let Some(summary) = summary {
            details.push(("summary".to_string(), output::s(summary)));
        }
        return Err(LoginError {
            code: "login-http-error",
            message: format!(
                "gemini-login: userinfo request failed (HTTP {http_status}) at {GOOGLE_USERINFO_URL}"
            ),
            details: Some(output::obj_dynamic(details)),
            exit_code: 3,
        });
    }

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|_| LoginError {
        code: "login-invalid-json",
        message: "gemini-login: userinfo endpoint returned invalid JSON".to_string(),
        details: Some(output::obj(vec![(
            "endpoint",
            output::s(GOOGLE_USERINFO_URL),
        )])),
        exit_code: 4,
    })?;
    Ok(json)
}

fn has_refresh_token(auth_file: &Path) -> bool {
    let value = match gemini_core::json::read_json(auth_file) {
        Ok(value) => value,
        Err(_) => return false,
    };
    refresh_token_from_json(&value).is_some()
}

fn access_token_from_json(value: &serde_json::Value) -> Option<String> {
    gemini_core::json::string_at(value, &["tokens", "access_token"])
        .or_else(|| gemini_core::json::string_at(value, &["access_token"]))
}

fn refresh_token_from_json(value: &serde_json::Value) -> Option<String> {
    gemini_core::json::string_at(value, &["tokens", "refresh_token"])
        .or_else(|| gemini_core::json::string_at(value, &["refresh_token"]))
}

fn split_http_status_marker(raw: &str) -> (String, u16) {
    let marker = "__HTTP_STATUS__:";
    if let Some(index) = raw.rfind(marker) {
        let body = raw[..index]
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string();
        let status_raw = raw[index + marker.len()..].trim();
        let status = status_raw.parse::<u16>().unwrap_or(0);
        (body, status)
    } else {
        (raw.to_string(), 0)
    }
}

fn http_error_summary(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let mut parts = Vec::new();

    if let Some(error) = value.get("error") {
        if let Some(error_str) = error.as_str() {
            if !error_str.is_empty() {
                parts.push(error_str.to_string());
            }
        } else if let Some(error_obj) = error.as_object() {
            if let Some(status) = error_obj.get("status").and_then(|value| value.as_str())
                && !status.is_empty()
            {
                parts.push(status.to_string());
            }
            if let Some(message) = error_obj.get("message").and_then(|value| value.as_str())
                && !message.is_empty()
            {
                parts.push(message.to_string());
            }
        }
    }

    if let Some(desc) = value
        .get("error_description")
        .and_then(|value| value.as_str())
        && !desc.is_empty()
    {
        parts.push(desc.to_string());
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(": "))
    }
}

fn env_timeout(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default)
}

fn emit_login_error(
    output_json: bool,
    code: &str,
    message: String,
    details: Option<output::JsonValue>,
) {
    if output_json {
        let _ = output::emit_error("auth login", code, message, details);
    } else {
        eprintln!("{message}");
    }
}

fn resolve_method(
    api_key: bool,
    device_code: bool,
) -> std::result::Result<LoginMethod, ErrorTriplet> {
    if api_key && device_code {
        return Err((
            64,
            "gemini-login: --api-key cannot be combined with --device-code".to_string(),
            Some(output::obj(vec![
                ("api_key", output::b(true)),
                ("device_code", output::b(true)),
            ])),
        ));
    }

    if api_key {
        return Ok(LoginMethod::ApiKey);
    }
    if device_code {
        return Ok(LoginMethod::GeminiDeviceCode);
    }
    Ok(LoginMethod::GeminiBrowser)
}

type ErrorTriplet = (i32, String, Option<output::JsonValue>);

impl LoginMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::GeminiBrowser => "gemini-browser",
            Self::GeminiDeviceCode => "gemini-device-code",
            Self::ApiKey => "api-key",
        }
    }

    fn provider(self) -> &'static str {
        match self {
            Self::GeminiBrowser | Self::GeminiDeviceCode => "gemini",
            Self::ApiKey => "gemini-api",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        LoginMethod, access_token_from_json, backup_auth_file, env_timeout, fetch_google_userinfo,
        has_refresh_token, http_error_summary, refresh_token_from_json, resolve_method,
        restore_auth_backup, run, run_api_key_login, run_gemini_interactive_login,
        run_oauth_interactive_login, run_oauth_session_check, run_with_json,
        split_http_status_marker,
    };

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        match LOCK.get_or_init(|| Mutex::new(())).lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    struct EnvGuard {
        key: String,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: tests serialize process environment mutations with `env_lock`.
            unsafe { std::env::set_var(key, value) };
            Self {
                key: key.to_string(),
                old,
            }
        }

        fn unset(key: &str) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: tests serialize process environment mutations with `env_lock`.
            unsafe { std::env::remove_var(key) };
            Self {
                key: key.to_string(),
                old,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.old.take() {
                // SAFETY: tests serialize process environment mutations with `env_lock`.
                unsafe { std::env::set_var(&self.key, value) };
            } else {
                // SAFETY: tests serialize process environment mutations with `env_lock`.
                unsafe { std::env::remove_var(&self.key) };
            }
        }
    }

    fn prepend_path(dir: &Path) -> EnvGuard {
        let mut value = dir.display().to_string();
        if let Ok(path) = std::env::var("PATH")
            && !path.is_empty()
        {
            value.push(':');
            value.push_str(&path);
        }
        EnvGuard::set("PATH", &value)
    }

    #[cfg(unix)]
    fn write_exe(path: &Path, content: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, content).expect("write executable");
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(not(unix))]
    fn write_exe(path: &Path, content: &str) {
        fs::write(path, content).expect("write executable");
    }

    fn write_script(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        write_exe(&path, content);
        path
    }

    fn curl_success_script() -> &'static str {
        r#"#!/bin/sh
set -eu
cat <<'EOF'
{"email":"alpha@example.com"}
__HTTP_STATUS__:200
EOF
"#
    }

    fn curl_http_error_script() -> &'static str {
        r#"#!/bin/sh
set -eu
cat <<'EOF'
{"error":{"status":"UNAUTHENTICATED","message":"token expired"},"error_description":"refresh needed"}
__HTTP_STATUS__:401
EOF
"#
    }

    fn curl_invalid_json_script() -> &'static str {
        r#"#!/bin/sh
set -eu
cat <<'EOF'
not-json
__HTTP_STATUS__:200
EOF
"#
    }

    fn curl_exit_failure_script() -> &'static str {
        r#"#!/bin/sh
exit 9
"#
    }

    #[test]
    fn run_delegates_to_run_with_json_non_json_mode() {
        let _lock = env_lock();
        let _api = EnvGuard::set("GEMINI_API_KEY", "dummy");
        let _google = EnvGuard::unset("GOOGLE_API_KEY");
        assert_eq!(run(true, false), 0);
    }

    #[test]
    fn run_with_json_reports_invalid_usage_for_conflicting_flags() {
        let _lock = env_lock();
        assert_eq!(run_with_json(true, true, true), 64);
    }

    #[test]
    fn run_api_key_login_json_errors_when_keys_are_missing() {
        let _lock = env_lock();
        let _api = EnvGuard::set("GEMINI_API_KEY", "");
        let _google = EnvGuard::set("GOOGLE_API_KEY", "");
        assert_eq!(run_api_key_login(true), 64);
    }

    #[test]
    fn run_api_key_login_uses_google_api_key_when_gemini_key_missing() {
        let _lock = env_lock();
        let _api = EnvGuard::set("GEMINI_API_KEY", "");
        let _google = EnvGuard::set("GOOGLE_API_KEY", "google-key");
        assert_eq!(run_api_key_login(true), 0);
    }

    #[test]
    fn run_oauth_session_check_missing_auth_file_returns_error() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let auth_file = temp.path().join("missing-auth.json");
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_session_check(LoginMethod::GeminiBrowser, true), 1);
    }

    #[test]
    fn run_oauth_session_check_invalid_auth_json_returns_error() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let auth_file = temp.path().join("oauth.json");
        fs::write(&auth_file, "{invalid").expect("write auth");
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_session_check(LoginMethod::GeminiBrowser, true), 1);
    }

    #[test]
    fn run_oauth_session_check_missing_access_token_returns_error() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let auth_file = temp.path().join("oauth.json");
        fs::write(&auth_file, "{}").expect("write auth");
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_session_check(LoginMethod::GeminiBrowser, true), 2);
    }

    #[test]
    fn run_oauth_session_check_http_error_returns_error() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(&bin_dir, "curl", curl_http_error_script());

        let auth_file = temp.path().join("oauth.json");
        fs::write(&auth_file, r#"{"access_token":"tok"}"#).expect("write auth");
        let _path = prepend_path(&bin_dir);
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_session_check(LoginMethod::GeminiBrowser, true), 3);
    }

    #[test]
    fn run_oauth_session_check_invalid_userinfo_json_returns_error() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(&bin_dir, "curl", curl_invalid_json_script());

        let auth_file = temp.path().join("oauth.json");
        fs::write(&auth_file, r#"{"access_token":"tok"}"#).expect("write auth");
        let _path = prepend_path(&bin_dir);
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_session_check(LoginMethod::GeminiBrowser, true), 4);
    }

    #[test]
    fn run_oauth_session_check_success_supports_nested_tokens() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(&bin_dir, "curl", curl_success_script());

        let auth_file = temp.path().join("oauth.json");
        fs::write(
            &auth_file,
            r#"{"tokens":{"access_token":"tok","refresh_token":"refresh-token"}}"#,
        )
        .expect("write auth");
        let _path = prepend_path(&bin_dir);
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_session_check(LoginMethod::GeminiBrowser, true), 0);
    }

    #[test]
    fn run_oauth_interactive_login_success_device_code_returns_zero() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(&bin_dir, "curl", curl_success_script());
        write_script(
            &bin_dir,
            "gemini",
            r#"#!/bin/sh
set -eu
[ "${NO_BROWSER:-}" = "true" ]
cat > "$GEMINI_AUTH_FILE" <<'EOF'
{"access_token":"new-token"}
EOF
"#,
        );

        let auth_file = temp.path().join("oauth.json");
        fs::write(&auth_file, r#"{"access_token":"old-token"}"#).expect("write auth");
        let _path = prepend_path(&bin_dir);
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(
            run_oauth_interactive_login(LoginMethod::GeminiDeviceCode),
            0
        );
        let updated = fs::read_to_string(&auth_file).expect("read auth");
        assert!(updated.contains("new-token"));
    }

    #[test]
    fn run_oauth_interactive_login_non_zero_status_restores_backup() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(
            &bin_dir,
            "gemini",
            r#"#!/bin/sh
set -eu
cat > "$GEMINI_AUTH_FILE" <<'EOF'
{"access_token":"new-token"}
EOF
exit 7
"#,
        );

        let auth_file = temp.path().join("oauth.json");
        let original = r#"{"access_token":"old-token"}"#;
        fs::write(&auth_file, original).expect("write auth");
        let _path = prepend_path(&bin_dir);
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_interactive_login(LoginMethod::GeminiBrowser), 7);
        assert_eq!(fs::read_to_string(&auth_file).expect("read auth"), original);
    }

    #[test]
    fn run_oauth_interactive_login_missing_token_restores_backup() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(
            &bin_dir,
            "gemini",
            r#"#!/bin/sh
set -eu
cat > "$GEMINI_AUTH_FILE" <<'EOF'
{}
EOF
"#,
        );

        let auth_file = temp.path().join("oauth.json");
        let original = r#"{"access_token":"old-token"}"#;
        fs::write(&auth_file, original).expect("write auth");
        let _path = prepend_path(&bin_dir);
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_interactive_login(LoginMethod::GeminiBrowser), 2);
        assert_eq!(fs::read_to_string(&auth_file).expect("read auth"), original);
    }

    #[test]
    fn run_oauth_interactive_login_userinfo_error_restores_backup() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(&bin_dir, "curl", curl_http_error_script());
        write_script(
            &bin_dir,
            "gemini",
            r#"#!/bin/sh
set -eu
cat > "$GEMINI_AUTH_FILE" <<'EOF'
{"access_token":"new-token"}
EOF
"#,
        );

        let auth_file = temp.path().join("oauth.json");
        let original = r#"{"access_token":"old-token"}"#;
        fs::write(&auth_file, original).expect("write auth");
        let _path = prepend_path(&bin_dir);
        let _auth = EnvGuard::set("GEMINI_AUTH_FILE", &auth_file.display().to_string());
        assert_eq!(run_oauth_interactive_login(LoginMethod::GeminiBrowser), 3);
        assert_eq!(fs::read_to_string(&auth_file).expect("read auth"), original);
    }

    #[test]
    fn run_gemini_interactive_login_errors_when_auth_file_not_created() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");
        write_script(
            &bin_dir,
            "gemini",
            r#"#!/bin/sh
exit 0
"#,
        );
        let _path = prepend_path(&bin_dir);
        let auth_file = temp.path().join("missing-output.json");
        let err = run_gemini_interactive_login(LoginMethod::GeminiBrowser, &auth_file)
            .expect_err("missing output file should fail");
        assert_eq!(err.code, "auth-file-not-found");
        assert_eq!(err.exit_code, 1);
    }

    #[test]
    fn fetch_google_userinfo_handles_command_failures_and_invalid_json() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin");

        write_script(&bin_dir, "curl", curl_exit_failure_script());
        let _path = prepend_path(&bin_dir);
        let request_err =
            fetch_google_userinfo("token").expect_err("non-zero curl exit should be an error");
        assert_eq!(request_err.code, "login-request-failed");
        assert_eq!(request_err.exit_code, 3);

        write_script(&bin_dir, "curl", curl_invalid_json_script());
        let invalid_json_err =
            fetch_google_userinfo("token").expect_err("invalid payload should fail");
        assert_eq!(invalid_json_err.code, "login-invalid-json");
        assert_eq!(invalid_json_err.exit_code, 4);
    }

    #[test]
    fn split_http_status_marker_and_error_summary_are_stable() {
        let (body, status) = split_http_status_marker("{\"ok\":true}\n__HTTP_STATUS__:200");
        assert_eq!(body, "{\"ok\":true}");
        assert_eq!(status, 200);

        let (body_without_marker, status_without_marker) = split_http_status_marker("plain-body");
        assert_eq!(body_without_marker, "plain-body");
        assert_eq!(status_without_marker, 0);

        let summary = http_error_summary(
            r#"{"error":{"status":"UNAUTHENTICATED","message":"token expired"},"error_description":"reauth"}"#,
        );
        assert_eq!(
            summary,
            Some("UNAUTHENTICATED: token expired: reauth".to_string())
        );
    }

    #[test]
    fn env_timeout_and_token_helpers_cover_defaults_and_nested_values() {
        let _lock = env_lock();
        let _timeout = EnvGuard::set("GEMINI_LOGIN_CURL_MAX_TIME_SECONDS", "11");
        assert_eq!(env_timeout("GEMINI_LOGIN_CURL_MAX_TIME_SECONDS", 8), 11);
        assert_eq!(env_timeout("GEMINI_LOGIN_CURL_UNKNOWN", 5), 5);

        let nested =
            json!({"tokens":{"access_token":"nested-access","refresh_token":"nested-refresh"}});
        assert_eq!(
            access_token_from_json(&nested),
            Some("nested-access".to_string())
        );
        assert_eq!(
            refresh_token_from_json(&nested),
            Some("nested-refresh".to_string())
        );

        let top_level = json!({"access_token":"top-access","refresh_token":"top-refresh"});
        assert_eq!(
            access_token_from_json(&top_level),
            Some("top-access".to_string())
        );
        assert_eq!(
            refresh_token_from_json(&top_level),
            Some("top-refresh".to_string())
        );
    }

    #[test]
    fn backup_restore_and_refresh_detection_behave_as_expected() {
        let _lock = env_lock();
        let temp = TempDir::new().expect("temp dir");
        let auth_file = temp.path().join("oauth.json");

        assert_eq!(
            backup_auth_file(&auth_file).expect("backup missing file"),
            None
        );
        assert_eq!(has_refresh_token(&auth_file), false);

        fs::write(&auth_file, r#"{"refresh_token":"refresh"}"#).expect("write auth");
        assert_eq!(has_refresh_token(&auth_file), true);

        let backup = backup_auth_file(&auth_file).expect("backup existing file");
        fs::write(&auth_file, r#"{"access_token":"mutated"}"#).expect("mutate auth");
        restore_auth_backup(&auth_file, backup.as_deref()).expect("restore backup");
        assert_eq!(
            fs::read_to_string(&auth_file).expect("read restored auth"),
            r#"{"refresh_token":"refresh"}"#
        );

        restore_auth_backup(&auth_file, None).expect("remove backup target");
        assert_eq!(auth_file.exists(), false);
    }

    #[test]
    fn resolve_method_defaults_to_gemini_browser() {
        assert_eq!(
            resolve_method(false, false).expect("method"),
            LoginMethod::GeminiBrowser
        );
    }

    #[test]
    fn resolve_method_selects_device_code_and_api_key() {
        assert_eq!(
            resolve_method(false, true).expect("method"),
            LoginMethod::GeminiDeviceCode
        );
        assert_eq!(
            resolve_method(true, false).expect("method"),
            LoginMethod::ApiKey
        );
    }

    #[test]
    fn resolve_method_rejects_conflicting_flags() {
        let err = resolve_method(true, true).expect_err("conflict should fail");
        assert_eq!(err.0, 64);
        assert!(err.1.contains("--api-key"));
    }

    #[test]
    fn login_method_strings_and_providers_are_stable() {
        assert_eq!(LoginMethod::GeminiBrowser.as_str(), "gemini-browser");
        assert_eq!(LoginMethod::GeminiDeviceCode.as_str(), "gemini-device-code");
        assert_eq!(LoginMethod::ApiKey.as_str(), "api-key");

        assert_eq!(LoginMethod::GeminiBrowser.provider(), "gemini");
        assert_eq!(LoginMethod::GeminiDeviceCode.provider(), "gemini");
        assert_eq!(LoginMethod::ApiKey.provider(), "gemini-api");
    }
}
