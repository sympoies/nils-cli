use nils_common::env as shared_env;
use serde_json::Value;
use std::process::{Command, Stdio};

const DEFAULT_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenSource {
    AccessTokenEnv,
    CredentialsJsonEnv,
    Keychain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessToken {
    pub value: String,
    pub source: TokenSource,
    pub plan: Option<&'static str>,
}

pub fn resolve_access_token() -> Option<AccessToken> {
    if let Some(value) = shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN") {
        return Some(AccessToken {
            plan: active_plan(&value),
            value,
            source: TokenSource::AccessTokenEnv,
        });
    }

    if let Some(raw) = shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_CREDENTIALS_JSON")
        && let Some(token) = credentials_login(&raw, TokenSource::CredentialsJsonEnv)
    {
        return Some(token);
    }

    if shared_env::env_truthy("CLAUDE_PROMPT_SEGMENT_KEYCHAIN_DISABLED") {
        return None;
    }

    keychain_credentials().and_then(|raw| credentials_login(&raw, TokenSource::Keychain))
}

fn active_plan(token: &str) -> Option<&'static str> {
    use crate::auth::store;

    let path = store::credentials_file()?;
    let credentials = store::read_json_object(&path, "active-credentials-invalid").ok()??;
    let oauth = credentials.get("claudeAiOauth")?.as_object()?;
    (oauth.get("accessToken")?.as_str()?.trim() == token)
        .then(|| crate::rate_limits::display::plan(oauth))
        .flatten()
}

fn credentials_login(raw: &str, source: TokenSource) -> Option<AccessToken> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let oauth = value.get("claudeAiOauth")?.as_object()?;
    let token = oauth.get("accessToken")?.as_str()?.trim();
    if token.is_empty() || token == "null" {
        return None;
    }
    Some(AccessToken {
        value: token.to_string(),
        source,
        plan: crate::rate_limits::display::plan(oauth),
    })
}

fn keychain_credentials() -> Option<String> {
    let service = shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_KEYCHAIN_SERVICE")
        .unwrap_or_else(|| DEFAULT_KEYCHAIN_SERVICE.to_string());
    let output = Command::new("security")
        .arg("find-generic-password")
        .arg("-s")
        .arg(service)
        .arg("-w")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_token_from_credentials_json_reads_claude_oauth_token() {
        let raw = r#"{"claudeAiOauth":{"accessToken":" token-123 "}}"#;
        assert_eq!(
            credentials_login(raw, TokenSource::CredentialsJsonEnv).map(|login| login.value),
            Some("token-123".to_string())
        );
    }

    #[test]
    fn access_token_from_credentials_json_rejects_missing_or_empty_tokens() {
        for raw in [
            "{}",
            r#"{"claudeAiOauth":{}}"#,
            r#"{"claudeAiOauth":{"accessToken":""}}"#,
            "not-json",
        ] {
            assert_eq!(
                credentials_login(raw, TokenSource::CredentialsJsonEnv).map(|login| login.value),
                None,
                "raw={raw}"
            );
        }
    }
}
