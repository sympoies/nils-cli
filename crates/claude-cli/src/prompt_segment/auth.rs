use crate::auth::store::{self, Identity};
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
}

pub fn resolve_access_token() -> Option<AccessToken> {
    if let Some(value) = shared_env::env_non_empty("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN") {
        return Some(AccessToken {
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

fn active_account(token: &str) -> Option<serde_json::Map<String, Value>> {
    let path = store::credentials_file()?;
    let credentials = store::read_json_object(&path, "active-credentials-invalid").ok()??;
    let oauth = credentials.get("claudeAiOauth")?.as_object()?;
    if store::non_empty_str(oauth.get("accessToken"))?.trim() != token {
        return None;
    }
    let config = store::global_config_file()?;
    let mut config = store::read_json_object(&config, "active-config-invalid").ok()??;
    Some(store::take_object(&mut config, "oauthAccount"))
}

/// Prefer the saved profile's name, not fields containing credentials.
pub(super) fn account_name(token: &str) -> Option<String> {
    let active = active_account(token);
    let identity = active.as_ref().and_then(Identity::from_account);
    if let Some(name) = store::list_profiles()
        .unwrap_or_default()
        .into_iter()
        .find(|name| {
            store::read_profile(name).is_ok_and(|profile| {
                store::non_empty_str(profile.oauth.get("accessToken")).map(str::trim) == Some(token)
                    || identity
                        .as_ref()
                        .zip(profile.identity())
                        .is_some_and(|(active, saved)| active.same_account(&saved))
            })
        })
    {
        return Some(name);
    }

    // Only use a local account label after matching the token to its login.
    let active = active?;
    if let Some(name) = store::config_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join(store::ACCOUNT_MARKER)).ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| store::is_account_dir_name(name))
    {
        return Some(name);
    }
    let (name, domain) = store::non_empty_str(active.get("emailAddress"))?
        .trim()
        .split_once('@')?;
    (!name.is_empty()
        && !domain.is_empty()
        && !name.chars().any(|ch| ch.is_control() || ch.is_whitespace()))
    .then(|| name.to_owned())
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
