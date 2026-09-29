//! Access-only remote auth for Claude Code over the shared
//! `nils_common::provider_runtime::remote` transport.

use nils_common::cli_contract::exit;
use nils_common::diag_output;
use nils_common::provider_runtime::remote::{self, AccessOnlyAdapter, RemoteSelector};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

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
            RemoteSelector::All => command.push("--all".to_string()),
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

/// The `--all` export: `{"current": <name|null>, "profiles": [<profile>...]}`.
struct ClaudeRemoteAll;

impl AccessOnlyAdapter for ClaudeRemoteAll {
    fn log_prefix(&self) -> &str {
        ClaudeRemote.log_prefix()
    }

    fn export_command(&self, _selector: &RemoteSelector, refresh: bool) -> Vec<String> {
        ClaudeRemote.export_command(&RemoteSelector::All, refresh)
    }

    fn sanitize_access_only(&self, value: Value) -> Value {
        sanitize_all(value)
    }

    /// Every exported profile carries an access token, and there is at least one.
    fn has_access_token(&self, value: &Value) -> bool {
        value
            .get("profiles")
            .and_then(Value::as_array)
            .is_some_and(|profiles| {
                !profiles.is_empty()
                    && profiles
                        .iter()
                        .all(|profile| ClaudeRemote.has_access_token(profile))
            })
    }
}

/// Sanitize every profile of an `--all` export and keep a valid `current`.
fn sanitize_all(value: Value) -> Value {
    let Value::Object(mut object) = value else {
        return Value::Object(Map::new());
    };
    let profiles: Vec<Value> = match object.remove("profiles") {
        Some(Value::Array(profiles)) => profiles.into_iter().map(sanitize).collect(),
        _ => Vec::new(),
    };
    let current = match object.remove("current") {
        Some(Value::String(name)) if store::validate_profile_name(&name).is_ok() => {
            Value::String(name)
        }
        _ => Value::Null,
    };
    let mut sanitized = Map::new();
    sanitized.insert("current".to_string(), current);
    sanitized.insert("profiles".to_string(), Value::Array(profiles));
    Value::Object(sanitized)
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

fn selector(name: Option<&str>, current: bool, all: bool) -> AuthResult<RemoteSelector> {
    match (name, current, all) {
        (Some(name), false, false) => {
            store::validate_profile_name(name)?;
            Ok(RemoteSelector::Name(name.to_string()))
        }
        (None, true, false) => Ok(RemoteSelector::Current),
        (None, false, true) => Ok(RemoteSelector::All),
        _ => Err(AuthError::usage(
            "selector-required",
            "pass exactly one of --name, --current, or --all",
        )),
    }
}

fn profile_payload(name: String) -> AuthResult<Value> {
    let profile = store::read_profile(&name)?;
    let mut payload = Map::new();
    payload.insert("profile".to_string(), Value::String(name));
    payload.insert("claudeAiOauth".to_string(), Value::Object(profile.oauth));
    payload.insert("oauthAccount".to_string(), Value::Object(profile.account));
    Ok(sanitize(Value::Object(payload)))
}

/// Print the access-only payload of a stored profile for SSH transport.
///
/// With `all`, every profile is printed in one payload together with the
/// current default, so a replica needs a single SSH round trip.
pub fn export(name: Option<&str>, current: bool, all: bool, access_only: bool) -> i32 {
    let result = (|| -> AuthResult<Value> {
        if !access_only {
            return Err(AuthError::usage(
                "access-only-required",
                "--access-only is required",
            ));
        }
        let name = match selector(name, current, all)? {
            RemoteSelector::Name(name) => name,
            RemoteSelector::Current => store::read_current()?.ok_or_else(|| {
                AuthError::data("current-not-set", "no current profile is selected")
            })?,
            RemoteSelector::All => {
                let profiles = store::list_profiles()?
                    .into_iter()
                    .map(profile_payload)
                    .collect::<AuthResult<Vec<_>>>()?;
                let mut payload = Map::new();
                let current = store::read_current()?.map_or(Value::Null, Value::String);
                payload.insert("current".to_string(), current);
                payload.insert("profiles".to_string(), Value::Array(profiles));
                return Ok(Value::Object(payload));
            }
        };
        profile_payload(name)
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
    pub all: bool,
    /// Accounts directory for `all`: one config dir per profile.
    pub into: Option<&'a Path>,
    pub access_only: bool,
    pub write_active: bool,
    pub keychain: keychain::Mode,
    pub output_json: bool,
}

/// Pull an access-only login from the authority and make it the active login.
///
/// With `all`, every profile is instead projected into its own config dir
/// under `into` (see [`pull_all`]).
pub fn pull(options: &PullOptions<'_>) -> i32 {
    if options.all {
        return pull_all(options);
    }
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
        if options.into.is_some() {
            return Err(AuthError::usage(
                "into-requires-all",
                "--into is only used with --all",
            ));
        }
        let selector = selector(options.name, options.current, false)?;
        let export = remote::fetch_access_only(&ClaudeRemote, options.ssh, &selector, false)
            .map_err(transport_error)?;
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

fn transport_error(failure: remote::RemotePullFailure) -> AuthError {
    AuthError {
        code: failure.code,
        message: failure.message,
        exit_code: failure.exit_code,
        details: failure.details,
    }
}

#[derive(Serialize)]
struct AccountResult {
    name: String,
    config_dir: String,
    written: bool,
    keychain: &'static str,
    expires_at: Option<i64>,
    has_refresh_token: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<AccountError>,
}

#[derive(Serialize)]
struct AccountError {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct PullAllResult {
    ssh: String,
    into: String,
    current: Option<String>,
    profiles: Vec<AccountResult>,
    pruned: Vec<String>,
}

/// Pull every authority profile access-only, one config dir per profile under
/// `into`, and prune the owned dirs of profiles that no longer exist.
fn pull_all(options: &PullOptions<'_>) -> i32 {
    let command = "auth remote pull";
    let result = (|| -> AuthResult<PullAllResult> {
        if !remote::is_valid_ssh_host(options.ssh) {
            return Err(AuthError::usage("invalid-ssh-host", "invalid ssh host"));
        }
        if !options.access_only {
            return Err(AuthError::usage(
                "access-only-required",
                "--access-only is required",
            ));
        }
        if options.write_active || options.name.is_some() || options.current {
            return Err(AuthError::usage(
                "selector-required",
                "--all cannot be combined with --name, --current, or --write-active",
            ));
        }
        let into = options.into.ok_or_else(|| {
            AuthError::usage("into-required", "--all needs --into <accounts-dir>")
        })?;
        let into = store::absolute_accounts_dir(into)?;
        let export =
            remote::fetch_access_only(&ClaudeRemoteAll, options.ssh, &RemoteSelector::All, false)
                .map_err(transport_error)?;
        let Value::Object(mut payload) = export.value else {
            return Err(AuthError::runtime(
                "remote-export-invalid-json",
                "remote export is not an object",
            ));
        };
        let current = match payload.remove("current") {
            Some(Value::String(name)) => Some(name),
            _ => None,
        };
        let Some(Value::Array(exported)) = payload.remove("profiles") else {
            return Err(AuthError::runtime(
                "remote-export-invalid-json",
                "remote export did not list profiles",
            ));
        };
        let mut names = BTreeSet::new();
        let mut entries = Vec::new();
        for entry in exported {
            let Value::Object(mut entry) = entry else {
                continue;
            };
            let name = match entry.remove("profile") {
                Some(Value::String(name)) if store::is_account_dir_name(&name) => name,
                _ => {
                    return Err(AuthError::data(
                        "remote-export-invalid-profile",
                        "remote export listed a profile without a valid name",
                    ));
                }
            };
            if !names.insert(name.clone()) {
                return Err(AuthError::data(
                    "remote-export-invalid-profile",
                    format!("remote export listed profile '{name}' twice"),
                ));
            }
            entries.push((name, entry));
        }

        let _lock = store::lock_accounts(&into)?;
        let mut profiles = Vec::new();
        for (name, mut entry) in entries {
            let oauth = store::take_object(&mut entry, "claudeAiOauth");
            let account = store::take_object(&mut entry, "oauthAccount");
            let written = if store::Identity::from_account(&account).is_none() {
                Err(AuthError::data(
                    "remote-export-missing-account",
                    "remote export did not include the account",
                ))
            } else {
                store::write_account_access_only(&into, &name, &oauth, &account, options.keychain)
            };
            let (keychain, error) = match written {
                Ok(written) => (written.keychain, None),
                Err(err) => (
                    "skipped",
                    Some(AccountError {
                        code: err.code,
                        message: err.message,
                    }),
                ),
            };
            profiles.push(AccountResult {
                config_dir: into.join(&name).display().to_string(),
                name,
                written: error.is_none(),
                keychain,
                expires_at: oauth.get("expiresAt").and_then(Value::as_i64),
                has_refresh_token: false,
                error,
            });
        }
        let pruned = store::prune_account_dirs(&into, &names)?;
        Ok(PullAllResult {
            ssh: options.ssh.to_string(),
            into: into.display().to_string(),
            current,
            profiles,
            pruned,
        })
    })();
    match result {
        Ok(result) => {
            let failed = result.profiles.iter().any(|profile| !profile.written);
            if options.output_json {
                if diag_output::emit_success_result(AUTH_SCHEMA_VERSION, command, &result).is_err()
                {
                    return exit::RUNTIME;
                }
            } else {
                for profile in &result.profiles {
                    match &profile.error {
                        None => println!(
                            "claude-remote-pull: {} -> {} (keychain {}){}",
                            profile.name,
                            profile.config_dir,
                            profile.keychain,
                            if result.current.as_deref() == Some(profile.name.as_str()) {
                                " [current]"
                            } else {
                                ""
                            }
                        ),
                        Some(error) => eprintln!(
                            "claude-remote-pull: {} failed ({}): {}",
                            profile.name, error.code, error.message
                        ),
                    }
                }
                for name in &result.pruned {
                    println!("claude-remote-pull: pruned {name}");
                }
            }
            if failed { exit::RUNTIME } else { exit::SUCCESS }
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
    fn sanitize_all_strips_refresh_tokens_and_invalid_current_names() {
        let value = json!({
            "current": "../max",
            "profiles": [{
                "profile": "max",
                "claudeAiOauth": { "accessToken": "a", "refreshToken": "r" },
                "oauthAccount": { "accountUuid": "u" }
            }],
            "extra": true
        });
        let sanitized = sanitize_all(value);
        assert_eq!(
            sanitized,
            json!({
                "current": null,
                "profiles": [{
                    "profile": "max",
                    "claudeAiOauth": { "accessToken": "a" },
                    "oauthAccount": { "accountUuid": "u" }
                }]
            })
        );
        assert!(ClaudeRemoteAll.has_access_token(&sanitized));
        assert!(!ClaudeRemoteAll.has_access_token(&json!({ "profiles": [] })));
        assert_eq!(
            ClaudeRemoteAll.export_command(&RemoteSelector::All, false),
            [
                "claude-cli",
                "auth",
                "remote",
                "export",
                "--all",
                "--access-only"
            ]
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
