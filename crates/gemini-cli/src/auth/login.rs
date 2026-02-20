use std::path::Path;
use std::process::Command;

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
    use super::{LoginMethod, resolve_method};

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
}
