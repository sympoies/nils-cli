use anyhow::Result;
use chrono::Utc;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::fs;
use crate::json;
use crate::paths;

pub fn run(args: &[String]) -> Result<i32> {
    let target_file = match resolve_target(args)? {
        Some(path) => path,
        None => return Ok(64),
    };

    if !target_file.is_file() {
        eprintln!("codex-refresh: {} not found", target_file.display());
        return Ok(1);
    }

    let value = match json::read_json(&target_file) {
        Ok(value) => value,
        Err(_) => {
            eprintln!(
                "codex-refresh: failed to read refresh token from {}",
                target_file.display()
            );
            return Ok(2);
        }
    };

    let refresh_token = refresh_token_from_json(&value);
    let refresh_token = match refresh_token {
        Some(token) => token,
        None => {
            eprintln!(
                "codex-refresh: failed to read refresh token from {}",
                target_file.display()
            );
            return Ok(2);
        }
    };

    let now_iso = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let client_id = std::env::var("CODEX_OAUTH_CLIENT_ID")
        .unwrap_or_else(|_| "app_EMoamEEZ73f0CkXaXp7hrann".to_string());

    let connect_timeout = env_timeout("CODEX_REFRESH_AUTH_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_REFRESH_AUTH_CURL_MAX_TIME_SECONDS", 8);

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(connect_timeout))
        .timeout(Duration::from_secs(max_time))
        .build()?;

    let response = client
        .post("https://auth.openai.com/oauth/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh_token.as_str()),
        ])
        .send();

    let response = match response {
        Ok(resp) => resp,
        Err(_) => {
            eprintln!(
                "codex-refresh: token endpoint request failed for {}",
                target_file.display()
            );
            return Ok(3);
        }
    };

    let status = response.status();
    let body = response.text().unwrap_or_default();

    if status.as_u16() != 200 {
        let summary = error_summary(&body);
        if let Some(summary) = summary {
            eprintln!(
                "codex-refresh: token endpoint failed (HTTP {}) for {}: {}",
                status.as_u16(),
                target_file.display(),
                summary
            );
        } else {
            eprintln!(
                "codex-refresh: token endpoint failed (HTTP {}) for {}",
                status.as_u16(),
                target_file.display()
            );
        }
        return Ok(3);
    }

    let response_json: Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("codex-refresh: token endpoint returned invalid JSON");
            return Ok(4);
        }
    };

    let merged = match merge_tokens(&value, &response_json, &now_iso) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("codex-refresh: failed to merge refreshed tokens");
            return Ok(5);
        }
    };

    let output = serde_json::to_vec(&merged)?;
    fs::write_atomic(&target_file, &output, fs::SECRET_FILE_MODE)?;

    let cache_dir = match paths::resolve_secret_cache_dir() {
        Some(dir) => dir,
        None => PathBuf::new(),
    };
    let timestamp_path = cache_dir.join(format!("{}.timestamp", file_name(&target_file)));
    if !cache_dir.as_os_str().is_empty() {
        fs::write_timestamp(&timestamp_path, Some(&now_iso))?;
    }

    if is_auth_file(&target_file) {
        let sync_rc = crate::auth::sync::run()?;
        if sync_rc != 0 {
            return Ok(6);
        }
    }

    println!("codex: refreshed {} at {}", target_file.display(), now_iso);
    Ok(0)
}

fn resolve_target(args: &[String]) -> Result<Option<PathBuf>> {
    if args.is_empty() {
        return Ok(Some(
            paths::resolve_auth_file().unwrap_or_else(|| PathBuf::from("auth.json")),
        ));
    }

    let secret_name = &args[0];
    if secret_name.is_empty() || secret_name.contains('/') || secret_name.contains("..") {
        eprintln!("codex-refresh: invalid secret file name: {secret_name}");
        return Ok(None);
    }

    let secret_dir = paths::resolve_secret_dir().unwrap_or_default();
    Ok(Some(secret_dir.join(secret_name)))
}

fn refresh_token_from_json(value: &Value) -> Option<String> {
    json::string_at(value, &["tokens", "refresh_token"])
        .or_else(|| json::string_at(value, &["refresh_token"]))
}

fn merge_tokens(base: &Value, refresh: &Value, now_iso: &str) -> Result<Value> {
    let mut root = base.as_object().cloned().unwrap_or_else(Map::new);
    let mut tokens = root
        .get("tokens")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_else(Map::new);

    if let Some(refresh_obj) = refresh.as_object() {
        for (key, value) in refresh_obj {
            tokens.insert(key.clone(), value.clone());
        }
    } else {
        return Err(anyhow::anyhow!("refresh payload is not object"));
    }

    root.insert("tokens".to_string(), Value::Object(tokens));
    root.insert(
        "last_refresh".to_string(),
        Value::String(now_iso.to_string()),
    );
    Ok(Value::Object(root))
}

fn error_summary(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let mut parts = Vec::new();

    if let Some(error) = value.get("error") {
        if error.is_object() {
            if let Some(code) = error.get("code").and_then(|v| v.as_str()) {
                if !code.is_empty() {
                    parts.push(code.to_string());
                }
            }
            if let Some(message) = error.get("message").and_then(|v| v.as_str()) {
                if !message.is_empty() {
                    parts.push(message.to_string());
                }
            }
        } else if let Some(error_str) = error.as_str() {
            if !error_str.is_empty() {
                parts.push(error_str.to_string());
            }
        }
    }

    if let Some(desc) = value.get("error_description").and_then(|v| v.as_str()) {
        if !desc.is_empty() {
            parts.push(desc.to_string());
        }
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

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth.json")
        .to_string()
}

fn is_auth_file(target: &Path) -> bool {
    if let Some(auth_file) = paths::resolve_auth_file() {
        if auth_file == target {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    struct EnvVarGuard {
        key: String,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key: key.to_string(),
                previous,
            }
        }

        fn remove(key: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self {
                key: key.to_string(),
                previous,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(&self.key, previous);
            } else {
                std::env::remove_var(&self.key);
            }
        }
    }

    #[test]
    fn auth_refresh_error_summary() {
        let body = r#"{"error":{"code":"invalid_grant","message":"Bad token"}}"#;
        let summary = error_summary(body).expect("summary");
        assert_eq!(summary, "invalid_grant: Bad token");
    }

    #[test]
    fn auth_refresh_merge_tokens() {
        let base: Value = serde_json::from_str(r#"{"tokens":{"access_token":"old"}}"#).unwrap();
        let refresh: Value =
            serde_json::from_str(r#"{"access_token":"new","refresh_token":"r1"}"#).unwrap();
        let merged = merge_tokens(&base, &refresh, "2025-01-20T00:00:00Z").unwrap();
        let tokens = merged.get("tokens").unwrap();
        assert_eq!(tokens.get("access_token").unwrap(), "new");
        assert_eq!(tokens.get("refresh_token").unwrap(), "r1");
        assert_eq!(merged.get("last_refresh").unwrap(), "2025-01-20T00:00:00Z");
    }

    #[test]
    fn auth_refresh_resolve_target_defaults_when_no_args() {
        let args: Vec<String> = Vec::new();
        let target = resolve_target(&args).unwrap().expect("target");
        assert!(!target.as_os_str().is_empty());
    }

    #[test]
    fn auth_refresh_resolve_target_rejects_invalid_secret_names() {
        for secret in ["", "a/b", "a..b", "../x"] {
            let args = vec![secret.to_string()];
            let target = resolve_target(&args).unwrap();
            assert!(target.is_none(), "expected None for secret={secret:?}");
        }
    }

    #[test]
    fn auth_refresh_resolve_target_joins_secret_name() {
        let secret_name = "my-secret.json";
        let args = vec![secret_name.to_string()];
        let target = resolve_target(&args).unwrap().expect("target");
        assert!(target.ends_with(secret_name));
    }

    #[test]
    fn auth_refresh_refresh_token_from_json_prefers_nested() {
        let value = serde_json::json!({
            "refresh_token": "top",
            "tokens": { "refresh_token": "nested" }
        });
        let token = refresh_token_from_json(&value).expect("token");
        assert_eq!(token, "nested");
    }

    #[test]
    fn auth_refresh_refresh_token_from_json_falls_back_to_top_level() {
        let value = serde_json::json!({ "refresh_token": "top" });
        let token = refresh_token_from_json(&value).expect("token");
        assert_eq!(token, "top");
    }

    #[test]
    fn auth_refresh_refresh_token_from_json_none_when_missing() {
        let value = serde_json::json!({ "tokens": { "access_token": "a1" } });
        assert!(refresh_token_from_json(&value).is_none());
    }

    #[test]
    fn auth_refresh_env_timeout_uses_default_when_missing_or_invalid() {
        let key = "CODEX_TEST_ENV_TIMEOUT_SECONDS_DEFAULT";
        let _guard = EnvVarGuard::remove(key);
        assert_eq!(env_timeout(key, 123), 123);

        let _guard = EnvVarGuard::set(key, "not-a-number");
        assert_eq!(env_timeout(key, 456), 456);

        let _guard = EnvVarGuard::set(key, "-1");
        assert_eq!(env_timeout(key, 789), 789);
    }

    #[test]
    fn auth_refresh_env_timeout_parses_value() {
        let key = "CODEX_TEST_ENV_TIMEOUT_SECONDS_PARSE";
        let _guard = EnvVarGuard::set(key, "42");
        assert_eq!(env_timeout(key, 1), 42);
    }

    #[test]
    fn auth_refresh_file_name_returns_basename() {
        let path = Path::new("my-auth.json");
        assert_eq!(file_name(path), "my-auth.json");
    }

    #[test]
    fn auth_refresh_file_name_defaults_when_missing() {
        let path = Path::new("");
        assert_eq!(file_name(path), "auth.json");
    }

    #[cfg(unix)]
    #[test]
    fn auth_refresh_file_name_defaults_when_non_utf8() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(vec![0xFF]));
        assert_eq!(file_name(&path), "auth.json");
    }
}
