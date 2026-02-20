use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::auth;
use crate::auth::output;

macro_rules! parse_json_text {
    ($raw:expr) => {{
        let tmp_path = crate::auth::temp_file_path("gemini-refresh-json");
        let parsed = (|| {
            std::fs::write(&tmp_path, $raw).ok()?;
            gemini_core::json::read_json(&tmp_path).ok()
        })();
        let _ = std::fs::remove_file(&tmp_path);
        parsed
    }};
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum RefreshOutputMode {
    Text,
    Json,
    Silent,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum AuthProvider {
    Google,
    OpenAi,
}

const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_DEFAULT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_DEFAULT_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";

pub fn run(args: &[String]) -> i32 {
    run_with_mode(args, RefreshOutputMode::Text)
}

pub fn run_with_json(args: &[String], output_json: bool) -> i32 {
    let mode = if output_json {
        RefreshOutputMode::Json
    } else {
        RefreshOutputMode::Text
    };
    run_with_mode(args, mode)
}

pub fn run_silent(args: &[String]) -> i32 {
    run_with_mode(args, RefreshOutputMode::Silent)
}

fn run_with_mode(args: &[String], output_mode: RefreshOutputMode) -> i32 {
    let output_json = output_mode == RefreshOutputMode::Json;
    let output_text = output_mode == RefreshOutputMode::Text;

    let target_file = match resolve_target(args, output_json) {
        Some(path) => path,
        None => return 64,
    };

    if !target_file.is_file() {
        if output_json {
            let _ = output::emit_error(
                "auth refresh",
                "target-not-found",
                format!("gemini-refresh: {} not found", target_file.display()),
                Some(output::obj(vec![(
                    "target_file",
                    output::s(target_file.display().to_string()),
                )])),
            );
        } else if output_text {
            eprintln!("gemini-refresh: {} not found", target_file.display());
        }
        return 1;
    }

    let mut value = match gemini_core::json::read_json(&target_file) {
        Ok(value) => value,
        Err(_) => {
            if output_json {
                let _ = output::emit_error(
                    "auth refresh",
                    "refresh-token-read-failed",
                    format!(
                        "gemini-refresh: failed to read refresh token from {}",
                        target_file.display()
                    ),
                    Some(output::obj(vec![(
                        "target_file",
                        output::s(target_file.display().to_string()),
                    )])),
                );
            } else if output_text {
                eprintln!(
                    "gemini-refresh: failed to read refresh token from {}",
                    target_file.display()
                );
            }
            return 2;
        }
    };

    let refresh_token = gemini_core::json::string_at(&value, &["tokens", "refresh_token"])
        .or_else(|| gemini_core::json::string_at(&value, &["refresh_token"]));

    let refresh_token = match refresh_token {
        Some(token) => token,
        None => {
            if output_json {
                let _ = output::emit_error(
                    "auth refresh",
                    "refresh-token-missing",
                    format!(
                        "gemini-refresh: failed to read refresh token from {}",
                        target_file.display()
                    ),
                    Some(output::obj(vec![(
                        "target_file",
                        output::s(target_file.display().to_string()),
                    )])),
                );
            } else if output_text {
                eprintln!(
                    "gemini-refresh: failed to read refresh token from {}",
                    target_file.display()
                );
            }
            return 2;
        }
    };

    let now_iso = auth::now_utc_iso();
    let provider = detect_provider(&value);
    let token_endpoint = match provider {
        AuthProvider::Google => GOOGLE_TOKEN_URL,
        AuthProvider::OpenAi => OPENAI_TOKEN_URL,
    };
    let client_id = resolve_client_id(provider, &value);
    let client_secret = std::env::var("GEMINI_OAUTH_CLIENT_SECRET")
        .ok()
        .filter(|value| !value.trim().is_empty());

    let connect_timeout = env_timeout("GEMINI_REFRESH_AUTH_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("GEMINI_REFRESH_AUTH_CURL_MAX_TIME_SECONDS", 8);

    let mut command = Command::new("curl");
    command
        .arg("-sS")
        .arg("--connect-timeout")
        .arg(connect_timeout.to_string())
        .arg("--max-time")
        .arg(max_time.to_string())
        .arg("-X")
        .arg("POST")
        .arg(token_endpoint)
        .arg("-H")
        .arg("Content-Type: application/x-www-form-urlencoded")
        .arg("--data-urlencode")
        .arg("grant_type=refresh_token")
        .arg("--data-urlencode")
        .arg(format!("client_id={client_id}"))
        .arg("--data-urlencode")
        .arg(format!("refresh_token={refresh_token}"));

    if let Some(client_secret) = client_secret.as_deref() {
        command
            .arg("--data-urlencode")
            .arg(format!("client_secret={client_secret}"));
    }

    let response = command
        .arg("-w")
        .arg("\n__HTTP_STATUS__:%{http_code}")
        .output();

    let response = match response {
        Ok(resp) => resp,
        Err(_) => {
            if output_json {
                let _ = output::emit_error(
                    "auth refresh",
                    "token-endpoint-request-failed",
                    format!(
                        "gemini-refresh: token endpoint request failed for {}",
                        target_file.display()
                    ),
                    Some(output::obj(vec![(
                        "target_file",
                        output::s(target_file.display().to_string()),
                    )])),
                );
            } else if output_text {
                eprintln!(
                    "gemini-refresh: token endpoint request failed for {}",
                    target_file.display()
                );
            }
            return 3;
        }
    };

    if !response.status.success() {
        if output_json {
            let _ = output::emit_error(
                "auth refresh",
                "token-endpoint-request-failed",
                format!(
                    "gemini-refresh: token endpoint request failed for {}",
                    target_file.display()
                ),
                Some(output::obj(vec![
                    ("target_file", output::s(target_file.display().to_string())),
                    ("endpoint", output::s(token_endpoint)),
                ])),
            );
        } else if output_text {
            eprintln!(
                "gemini-refresh: token endpoint request failed for {}",
                target_file.display()
            );
        }
        return 3;
    }

    let response_text = String::from_utf8_lossy(&response.stdout).to_string();
    let (body, http_status) = split_http_status_marker(&response_text);

    if http_status != 200 {
        let summary = error_summary(&body);
        if output_json {
            let mut details = vec![
                ("http_status", output::n(http_status as i64)),
                ("target_file", output::s(target_file.display().to_string())),
                ("endpoint", output::s(token_endpoint)),
            ];
            if let Some(summary) = summary.clone() {
                details.push(("summary", output::s(summary)));
            }
            let _ = output::emit_error(
                "auth refresh",
                "token-endpoint-failed",
                format!(
                    "gemini-refresh: token endpoint failed (HTTP {}) for {}",
                    http_status,
                    target_file.display()
                ),
                Some(output::obj_dynamic(
                    details
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value))
                        .collect(),
                )),
            );
        } else if output_text {
            if let Some(summary) = summary {
                eprintln!(
                    "gemini-refresh: token endpoint failed (HTTP {}) for {}: {}",
                    http_status,
                    target_file.display(),
                    summary
                );
            } else {
                eprintln!(
                    "gemini-refresh: token endpoint failed (HTTP {}) for {}",
                    http_status,
                    target_file.display()
                );
            }
        }
        return 3;
    }

    let response_json = match parse_json_text!(body.as_str()) {
        Some(value) => value,
        None => {
            if output_json {
                let _ = output::emit_error(
                    "auth refresh",
                    "token-endpoint-invalid-json",
                    "gemini-refresh: token endpoint returned invalid JSON",
                    None,
                );
            } else if output_text {
                eprintln!("gemini-refresh: token endpoint returned invalid JSON");
            }
            return 4;
        }
    };

    let merge_ok = merge_refreshed_tokens(
        &mut value,
        &response_json,
        &now_iso,
        &refresh_token,
        provider,
    );

    if !merge_ok {
        return merge_failed(output_json, output_text);
    }

    let serialized = value.to_string();
    if auth::write_atomic(&target_file, serialized.as_bytes(), auth::SECRET_FILE_MODE).is_err() {
        if output_json {
            let _ = output::emit_error(
                "auth refresh",
                "refresh-write-failed",
                format!(
                    "gemini-refresh: failed to write refreshed tokens to {}",
                    target_file.display()
                ),
                Some(output::obj(vec![(
                    "target_file",
                    output::s(target_file.display().to_string()),
                )])),
            );
        } else if output_text {
            eprintln!(
                "gemini-refresh: failed to write refreshed tokens to {}",
                target_file.display()
            );
        }
        return 1;
    }

    if let Some(cache_dir) = gemini_core::paths::resolve_secret_cache_dir() {
        let timestamp_path = cache_dir.join(format!("{}.timestamp", file_name(&target_file)));
        let _ = auth::write_timestamp(&timestamp_path, Some(&now_iso));
    }

    let mut synced = false;
    if is_auth_file(&target_file) {
        let sync_rc = crate::auth::sync::run_with_json(false);
        if sync_rc != 0 {
            if output_json {
                let _ = output::emit_error(
                    "auth refresh",
                    "sync-failed",
                    "gemini-refresh: failed to sync refreshed auth into matching secrets",
                    Some(output::obj(vec![(
                        "target_file",
                        output::s(target_file.display().to_string()),
                    )])),
                );
            }
            return 6;
        }
        synced = true;
    }

    if output_json {
        let _ = output::emit_result(
            "auth refresh",
            output::obj(vec![
                ("target_file", output::s(target_file.display().to_string())),
                ("refreshed", output::b(true)),
                ("synced", output::b(synced)),
                ("refreshed_at", output::s(now_iso)),
            ]),
        );
    } else if output_text {
        println!("gemini: refreshed {} at {}", target_file.display(), now_iso);
    }

    0
}

fn merge_failed(output_json: bool, output_text: bool) -> i32 {
    if output_json {
        let _ = output::emit_error(
            "auth refresh",
            "merge-failed",
            "gemini-refresh: failed to merge refreshed tokens",
            None,
        );
    } else if output_text {
        eprintln!("gemini-refresh: failed to merge refreshed tokens");
    }
    5
}

fn merge_refreshed_tokens(
    base: &mut Value,
    refresh: &Value,
    now_iso: &str,
    current_refresh_token: &str,
    provider: AuthProvider,
) -> bool {
    let google_subject = if provider == AuthProvider::Google {
        subject_from_json(base)
    } else {
        None
    };

    let Some(root_obj) = base.as_object_mut() else {
        return false;
    };

    if root_obj
        .get("tokens")
        .and_then(|token_value| token_value.as_object())
        .is_none()
    {
        root_obj.insert("tokens".to_string(), json!({}));
    }

    let Some(refresh_obj) = refresh.as_object() else {
        return false;
    };

    for (key, value) in refresh_obj {
        if let Some(tokens_obj) = root_obj
            .get_mut("tokens")
            .and_then(|token_value| token_value.as_object_mut())
        {
            tokens_obj.insert(key.clone(), value.clone());
        } else {
            return false;
        }
        root_obj.insert(key.clone(), value.clone());
    }

    if !refresh_obj.contains_key("refresh_token") {
        if let Some(tokens_obj) = root_obj
            .get_mut("tokens")
            .and_then(|token_value| token_value.as_object_mut())
        {
            tokens_obj.insert("refresh_token".to_string(), json!(current_refresh_token));
        } else {
            return false;
        }
        root_obj
            .entry("refresh_token".to_string())
            .or_insert_with(|| json!(current_refresh_token));
    }

    if let Some(expires_in) = refresh_obj
        .get("expires_in")
        .and_then(|value| value.as_i64())
    {
        let expiry_date = auth::now_epoch_seconds().saturating_add(expires_in) * 1000;
        root_obj.insert("expiry_date".to_string(), json!(expiry_date));
    }

    root_obj.insert("last_refresh".to_string(), json!(now_iso));

    if let Some(subject) = google_subject {
        if let Some(tokens_obj) = root_obj
            .get_mut("tokens")
            .and_then(|token_value| token_value.as_object_mut())
        {
            tokens_obj.insert("account_id".to_string(), json!(subject.clone()));
        } else {
            return false;
        }
        root_obj.insert("account_id".to_string(), json!(subject));
    }

    true
}

fn resolve_target(args: &[String], output_json: bool) -> Option<PathBuf> {
    if args.is_empty() {
        return Some(
            gemini_core::paths::resolve_auth_file().unwrap_or_else(|| PathBuf::from("auth.json")),
        );
    }

    let secret_name = &args[0];
    if secret_name.is_empty() || secret_name.contains('/') || secret_name.contains("..") {
        if output_json {
            let _ = output::emit_error(
                "auth refresh",
                "invalid-secret-file-name",
                format!("gemini-refresh: invalid secret file name: {secret_name}"),
                Some(output::obj(vec![("secret", output::s(secret_name))])),
            );
        } else {
            eprintln!("gemini-refresh: invalid secret file name: {secret_name}");
        }
        return None;
    }

    let secret_dir = gemini_core::paths::resolve_secret_dir().unwrap_or_default();
    Some(secret_dir.join(secret_name))
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

fn error_summary(body: &str) -> Option<String> {
    let value = parse_json_text!(body)?;
    let mut parts = Vec::new();

    if let Some(error) = value.get("error") {
        if error.is_object() {
            if let Some(code) = error.get("code").and_then(|v| v.as_str())
                && !code.is_empty()
            {
                parts.push(code.to_string());
            }
            if let Some(message) = error.get("message").and_then(|v| v.as_str())
                && !message.is_empty()
            {
                parts.push(message.to_string());
            }
        } else if let Some(error_str) = error.as_str()
            && !error_str.is_empty()
        {
            parts.push(error_str.to_string());
        }
    }

    if let Some(desc) = value.get("error_description").and_then(|v| v.as_str())
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

fn detect_provider(value: &Value) -> AuthProvider {
    if let Ok(raw) = std::env::var("GEMINI_OAUTH_PROVIDER") {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized == "google" || normalized == "gemini" {
            return AuthProvider::Google;
        }
        if normalized == "openai" {
            return AuthProvider::OpenAi;
        }
    }

    let payload = id_payload_from_json(value);
    let iss = payload
        .as_ref()
        .and_then(|payload| payload.get("iss"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if iss.contains("accounts.google.com") {
        return AuthProvider::Google;
    }

    let aud = payload
        .as_ref()
        .and_then(|payload| payload.get("aud"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if aud.ends_with(".apps.googleusercontent.com") {
        return AuthProvider::Google;
    }

    AuthProvider::OpenAi
}

fn resolve_client_id(provider: AuthProvider, value: &Value) -> String {
    if let Ok(raw) = std::env::var("GEMINI_OAUTH_CLIENT_ID")
        && !raw.trim().is_empty()
    {
        return raw;
    }

    if provider == AuthProvider::Google
        && let Some(aud) = id_payload_from_json(value).and_then(|payload| {
            payload
                .get("aud")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        && !aud.trim().is_empty()
    {
        return aud;
    }

    match provider {
        AuthProvider::Google => GOOGLE_DEFAULT_CLIENT_ID.to_string(),
        AuthProvider::OpenAi => OPENAI_DEFAULT_CLIENT_ID.to_string(),
    }
}

fn subject_from_json(value: &Value) -> Option<String> {
    id_payload_from_json(value)
        .and_then(|payload| {
            payload
                .get("sub")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .map(|subject| gemini_core::json::strip_newlines(&subject))
}

fn id_payload_from_json(value: &Value) -> Option<Value> {
    let token = gemini_core::json::string_at(value, &["tokens", "id_token"])
        .or_else(|| gemini_core::json::string_at(value, &["id_token"]))?;
    gemini_core::jwt::decode_payload_json(&token)
}

fn env_timeout(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json")
        .to_string()
}

fn is_auth_file(target: &Path) -> bool {
    if let Some(auth_file) = gemini_core::paths::resolve_auth_file()
        && auth_file == target
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        AuthProvider, detect_provider, env_timeout, error_summary, file_name, is_auth_file,
        merge_failed, resolve_client_id, resolve_target, split_http_status_marker,
    };
    use std::path::Path;

    #[test]
    fn split_http_status_extracts_marker() {
        let (body, status) = split_http_status_marker("{\"ok\":true}\n__HTTP_STATUS__:200");
        assert_eq!(body, "{\"ok\":true}");
        assert_eq!(status, 200);
    }

    #[test]
    fn split_http_status_without_marker_returns_zero_status() {
        let (body, status) = split_http_status_marker("{\"ok\":true}");
        assert_eq!(body, "{\"ok\":true}");
        assert_eq!(status, 0);
    }

    #[test]
    fn env_timeout_uses_default_when_missing_or_invalid() {
        let key = "GEMINI_TEST_ENV_TIMEOUT_SECONDS_DEFAULT";
        // SAFETY: test-scoped env mutation.
        unsafe { std::env::remove_var(key) };
        assert_eq!(env_timeout(key, 123), 123);

        // SAFETY: test-scoped env mutation.
        unsafe { std::env::set_var(key, "not-a-number") };
        assert_eq!(env_timeout(key, 456), 456);

        // SAFETY: test-scoped env cleanup.
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn file_name_defaults_when_missing() {
        assert_eq!(file_name(Path::new("")), "auth.json");
    }

    #[test]
    fn resolve_target_rejects_invalid_secret_name() {
        let args = vec!["../bad.json".to_string()];
        assert!(resolve_target(&args, false).is_none());
    }

    #[test]
    fn resolve_target_uses_default_auth_path_when_env_missing() {
        let key = "GEMINI_AUTH_FILE";
        let home_key = "HOME";
        let old = std::env::var_os(key);
        let old_home = std::env::var_os(home_key);
        let temp_home = std::env::temp_dir().join(format!(
            "nils-gemini-refresh-home-{}-{}",
            std::process::id(),
            super::auth::now_epoch_seconds()
        ));
        let _ = std::fs::create_dir_all(&temp_home);
        // SAFETY: test-scoped env mutation.
        unsafe { std::env::remove_var(key) };
        // SAFETY: test-scoped env mutation.
        unsafe { std::env::set_var(home_key, &temp_home) };
        let resolved = resolve_target(&[], false).expect("resolved path");
        assert!(resolved.ends_with("oauth_creds.json"));
        if let Some(value) = old {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::set_var(key, value) };
        } else {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::remove_var(key) };
        }
        if let Some(value) = old_home {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::set_var(home_key, value) };
        } else {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::remove_var(home_key) };
        }
        let _ = std::fs::remove_dir_all(temp_home);
    }

    #[test]
    fn error_summary_extracts_object_and_description() {
        let body = r#"{"error":{"code":"invalid_grant","message":"expired"},"error_description":"reauth"}"#;
        let summary = error_summary(body).expect("summary");
        assert!(summary.contains("invalid_grant"));
        assert!(summary.contains("expired"));
        assert!(summary.contains("reauth"));
    }

    #[test]
    fn error_summary_supports_string_error_field() {
        let body = r#"{"error":"bad_request"}"#;
        assert_eq!(error_summary(body).as_deref(), Some("bad_request"));
    }

    #[test]
    fn is_auth_file_matches_env_path() {
        let key = "GEMINI_AUTH_FILE";
        let old = std::env::var_os(key);
        // SAFETY: test-scoped env mutation.
        unsafe { std::env::set_var(key, "/tmp/gemini-auth.json") };
        assert!(is_auth_file(Path::new("/tmp/gemini-auth.json")));
        if let Some(value) = old {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::set_var(key, value) };
        } else {
            // SAFETY: test-scoped env restore.
            unsafe { std::env::remove_var(key) };
        }
    }

    #[test]
    fn merge_failed_always_returns_exit_code_five() {
        assert_eq!(merge_failed(false, true), 5);
        assert_eq!(merge_failed(true, false), 5);
    }

    #[test]
    fn detect_provider_prefers_google_issuer_and_audience() {
        let google: serde_json::Value = serde_json::json!({
            "id_token": "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJpc3MiOiJodHRwczovL2FjY291bnRzLmdvb2dsZS5jb20iLCJhdWQiOiJhYmMuYXBwcy5nb29nbGV1c2VyY29udGVudC5jb20ifQ.sig"
        });
        assert!(matches!(detect_provider(&google), AuthProvider::Google));

        let openai: serde_json::Value = serde_json::json!({
            "id_token": "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJpc3MiOiJodHRwczovL2F1dGgub3BlbmFpLmNvbSJ9.sig"
        });
        assert!(matches!(detect_provider(&openai), AuthProvider::OpenAi));
    }

    #[test]
    fn resolve_client_id_uses_google_audience_when_available() {
        let value: serde_json::Value = serde_json::json!({
            "id_token": "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJhdWQiOiJhYmMta2V5LmFwcHMuZ29vZ2xldXNlcmNvbnRlbnQuY29tIn0.sig"
        });
        let client_id = resolve_client_id(AuthProvider::Google, &value);
        assert_eq!(client_id, "abc-key.apps.googleusercontent.com");
    }
}
