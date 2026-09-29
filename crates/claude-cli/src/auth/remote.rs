//! Access-only remote auth for Claude Code over the shared
//! `nils_common::provider_runtime::remote` transport.

use nils_common::cli_contract::exit;
use nils_common::diag_output;
use nils_common::provider_runtime::remote::{self, AccessOnlyAdapter, RemoteSelector};
use serde::Serialize;
use serde_json::{Map, Value};

use super::AUTH_SCHEMA_VERSION;
use super::keychain;
use super::profile::emit_error;
use super::store::{self, AuthError, AuthResult};

struct ClaudeRemote;

impl AccessOnlyAdapter for ClaudeRemote {
    fn log_prefix(&self) -> &str {
        "claude-remote-pull"
    }

    fn export_command(&self, selector: &RemoteSelector, _refresh: bool) -> Vec<String> {
        let mut command = ["claude-cli", "auth", "remote", "export"]
            .map(str::to_string)
            .to_vec();
        match selector {
            RemoteSelector::Name(name) => {
                command.push("--name".to_string());
                command.push(name.clone());
            }
            RemoteSelector::Current => command.push("--current".to_string()),
        }
        command.push("--access-only".to_string());
        command
    }

    fn sanitize_access_only(&self, value: Value) -> Value {
        sanitize(value)
    }

    fn has_access_token(&self, value: &Value) -> bool {
        value
            .get("claudeAiOauth")
            .and_then(|oauth| store::non_empty_str(oauth.get("accessToken")))
            .is_some()
    }
}

/// Keep the profile name, the access fields of `claudeAiOauth`, and `oauthAccount`.
fn sanitize(value: Value) -> Value {
    let Value::Object(mut object) = value else {
        return Value::Object(Map::new());
    };
    let mut sanitized = Map::new();
    if let Some(Value::String(profile)) = object.remove("profile") {
        sanitized.insert("profile".to_string(), Value::String(profile));
    }
    let oauth = store::take_object(&mut object, "claudeAiOauth");
    sanitized.insert(
        "claudeAiOauth".to_string(),
        Value::Object(store::access_fields(&oauth)),
    );
    let account = store::take_object(&mut object, "oauthAccount");
    sanitized.insert("oauthAccount".to_string(), Value::Object(account));
    Value::Object(sanitized)
}

fn selector(name: Option<&str>, current: bool) -> AuthResult<RemoteSelector> {
    match (name, current) {
        (Some(name), false) => {
            store::validate_profile_name(name)?;
            Ok(RemoteSelector::Name(name.to_string()))
        }
        (None, true) => Ok(RemoteSelector::Current),
        _ => Err(AuthError::usage(
            "selector-required",
            "pass exactly one of --name or --current",
        )),
    }
}

/// Print the access-only payload of a stored profile for SSH transport.
pub fn export(name: Option<&str>, current: bool, access_only: bool) -> i32 {
    let result = (|| -> AuthResult<Value> {
        if !access_only {
            return Err(AuthError::usage(
                "access-only-required",
                "--access-only is required",
            ));
        }
        let name = match selector(name, current)? {
            RemoteSelector::Name(name) => name,
            RemoteSelector::Current => store::read_current()?.ok_or_else(|| {
                AuthError::data("current-not-set", "no current profile is selected")
            })?,
        };
        let profile = store::read_profile(&name)?;
        let mut payload = Map::new();
        payload.insert("profile".to_string(), Value::String(name));
        payload.insert("claudeAiOauth".to_string(), Value::Object(profile.oauth));
        payload.insert("oauthAccount".to_string(), Value::Object(profile.account));
        Ok(sanitize(Value::Object(payload)))
    })();
    match result {
        Ok(payload) => match serde_json::to_string(&payload) {
            Ok(text) => {
                println!("{text}");
                exit::SUCCESS
            }
            Err(_) => exit::RUNTIME,
        },
        Err(err) => {
            eprintln!("claude-remote-export: {}", err.message);
            err.exit_code
        }
    }
}

#[derive(Serialize)]
struct PullResult {
    ssh: String,
    profile: Option<String>,
    account_uuid: Option<String>,
    expires_at: Option<i64>,
    credentials_file: String,
    config_updated: bool,
    keychain: &'static str,
    has_refresh_token: bool,
}

pub struct PullOptions<'a> {
    pub ssh: &'a str,
    pub name: Option<&'a str>,
    pub current: bool,
    pub access_only: bool,
    pub write_active: bool,
    pub keychain: keychain::Mode,
    pub output_json: bool,
}

/// Pull an access-only login from the authority and make it the active login.
pub fn pull(options: &PullOptions<'_>) -> i32 {
    let command = "auth remote pull";
    let result = (|| -> AuthResult<PullResult> {
        if !remote::is_valid_ssh_host(options.ssh) {
            return Err(AuthError::usage("invalid-ssh-host", "invalid ssh host"));
        }
        if !options.access_only {
            return Err(AuthError::usage(
                "access-only-required",
                "--access-only is required",
            ));
        }
        if !options.write_active {
            return Err(AuthError::usage(
                "write-active-required",
                "--write-active is required",
            ));
        }
        let selector = selector(options.name, options.current)?;
        let export = remote::fetch_access_only(&ClaudeRemote, options.ssh, &selector, false)
            .map_err(|failure| AuthError {
                code: failure.code,
                message: failure.message,
                exit_code: failure.exit_code,
                details: failure.details,
            })?;
        let Value::Object(mut payload) = export.value else {
            return Err(AuthError::runtime(
                "remote-export-invalid-json",
                "remote export is not an object",
            ));
        };
        let profile = match payload.remove("profile") {
            Some(Value::String(profile)) => Some(profile),
            _ => None,
        };
        let oauth = store::take_object(&mut payload, "claudeAiOauth");
        let account = store::take_object(&mut payload, "oauthAccount");
        let identity = store::Identity::from_account(&account).ok_or_else(|| {
            AuthError::data(
                "remote-export-missing-account",
                "remote export did not include the account",
            )
        })?;
        let written = store::write_active_access_only(&oauth, &account, options.keychain)?;
        Ok(PullResult {
            ssh: options.ssh.to_string(),
            profile,
            account_uuid: Some(identity.account_uuid),
            expires_at: oauth.get("expiresAt").and_then(Value::as_i64),
            credentials_file: written.credentials_file.display().to_string(),
            config_updated: written.config_updated,
            keychain: written.keychain,
            has_refresh_token: false,
        })
    })();
    match result {
        Ok(result) => {
            if options.output_json {
                if diag_output::emit_success_result(AUTH_SCHEMA_VERSION, command, &result).is_err()
                {
                    return exit::RUNTIME;
                }
            } else {
                println!(
                    "claude-remote-pull: pulled access-only login '{}' from {} (keychain {})",
                    result.profile.as_deref().unwrap_or("?"),
                    result.ssh,
                    result.keychain
                );
            }
            exit::SUCCESS
        }
        Err(err) => emit_error(command, options.output_json, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn sanitize_keeps_only_access_fields_and_account() {
        let value = json!({
            "profile": "max",
            "claudeAiOauth": { "accessToken": "a", "refreshToken": "r", "expiresAt": 1 },
            "oauthAccount": { "accountUuid": "u" },
            "mcpOAuth": { "token": "m" }
        });
        assert_eq!(
            sanitize(value),
            json!({
                "profile": "max",
                "claudeAiOauth": { "accessToken": "a", "expiresAt": 1 },
                "oauthAccount": { "accountUuid": "u" }
            })
        );
    }

    #[test]
    fn export_command_selects_by_name_or_current() {
        assert_eq!(
            ClaudeRemote.export_command(&RemoteSelector::Current, false),
            [
                "claude-cli",
                "auth",
                "remote",
                "export",
                "--current",
                "--access-only"
            ]
        );
        assert_eq!(
            ClaudeRemote.export_command(&RemoteSelector::Name("team".to_string()), true),
            [
                "claude-cli",
                "auth",
                "remote",
                "export",
                "--name",
                "team",
                "--access-only"
            ]
        );
    }
}
