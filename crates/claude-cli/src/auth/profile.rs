//! Authority-side profile commands: save, use, current, refresh, auto-refresh.

use nils_common::cli_contract::exit;
use nils_common::diag_output;
use nils_common::env as shared_env;
use serde::Serialize;
use serde_json::{Map, Value};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::AUTH_SCHEMA_VERSION;
use super::keychain;
use super::store::{self, AuthError, AuthResult, Identity, Profile};
use crate::agent::oneshot::claude_binary;
use crate::process::{ProcessOutputError, output_with_limits_retry_io};

const REFRESH_MARGIN_ENV: &str = "CLAUDE_AUTH_REFRESH_MARGIN_SECONDS";
/// Refresh a profile once less than this much access-token lifetime is left.
const DEFAULT_REFRESH_MARGIN_SECONDS: i64 = 4 * 60 * 60;
const REFRESH_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_REFRESH_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Serialize)]
struct SaveResult {
    profile: String,
    account_uuid: String,
    replaced: bool,
}

#[derive(Serialize)]
struct UseResult {
    profile: String,
    account_uuid: Option<String>,
    credentials_file: String,
    config_updated: bool,
    keychain: &'static str,
}

#[derive(Serialize)]
struct CurrentResult {
    profile: Option<String>,
    account_uuid: Option<String>,
    organization_uuid: Option<String>,
    expires_at: Option<i64>,
    profiles: Vec<String>,
}

#[derive(Serialize)]
struct RefreshFailure {
    profile: String,
    code: &'static str,
    message: String,
}

#[derive(Serialize, Default)]
struct RefreshResult {
    refreshed: Vec<String>,
    skipped: Vec<String>,
    failed: Vec<RefreshFailure>,
}

pub fn save(name: &str, output_json: bool) -> i32 {
    let command = "auth save";
    let result = (|| -> AuthResult<SaveResult> {
        store::validate_profile_name(name)?;
        let _lock = store::lock_store()?;
        let login = store::read_active_login()?;
        if login.refresh_token().is_none() {
            return Err(AuthError::data(
                "active-login-access-only",
                "the active login has no refresh token; save a login made with /login",
            ));
        }
        let identity = login.identity().ok_or_else(|| {
            AuthError::data(
                "active-login-without-account",
                "the active login has no oauthAccount in the Claude Code config",
            )
        })?;
        let replaced = match store::read_profile(name) {
            Ok(existing) => {
                if !existing
                    .identity()
                    .is_some_and(|existing| existing.same_account(&identity))
                {
                    return Err(AuthError::data(
                        "profile-identity-mismatch",
                        format!("profile '{name}' belongs to a different account"),
                    ));
                }
                true
            }
            Err(err) if err.code == "profile-not-found" => false,
            Err(err) => return Err(err),
        };
        store::write_profile(name, &login)?;
        // The profile is now the only refresher: leave the source login access-only.
        store::write_active_access_only(&login.oauth, &login.account, keychain::Mode::Auto)?;
        Ok(SaveResult {
            profile: name.to_string(),
            account_uuid: identity.account_uuid,
            replaced,
        })
    })();
    finish(command, output_json, result, |result| {
        format!("claude-cli: saved profile '{}'", result.profile)
    })
}

pub fn use_profile(name: &str, output_json: bool) -> i32 {
    let command = "auth use";
    let result = (|| -> AuthResult<UseResult> {
        let _lock = store::lock_store()?;
        let profile = store::read_profile(name)?;
        let written = store::write_active_access_only(
            &profile.oauth,
            &profile.account,
            keychain::Mode::Auto,
        )?;
        store::write_current(name)?;
        Ok(UseResult {
            profile: name.to_string(),
            account_uuid: profile.identity().map(|identity| identity.account_uuid),
            credentials_file: written.credentials_file.display().to_string(),
            config_updated: written.config_updated,
            keychain: written.keychain,
        })
    })();
    finish(command, output_json, result, |result| {
        format!("claude-cli: using profile '{}'", result.profile)
    })
}

pub fn current(output_json: bool) -> i32 {
    let command = "auth current";
    let result = (|| -> AuthResult<CurrentResult> {
        let profiles = store::list_profiles()?;
        let Some(name) = store::read_current()? else {
            return Ok(CurrentResult {
                profile: None,
                account_uuid: None,
                organization_uuid: None,
                expires_at: None,
                profiles,
            });
        };
        let profile = store::read_profile(&name)?;
        let identity = profile.identity();
        Ok(CurrentResult {
            profile: Some(name),
            account_uuid: identity.as_ref().map(|id| id.account_uuid.clone()),
            organization_uuid: identity.map(|id| id.organization_uuid),
            expires_at: profile.expires_at_ms(),
            profiles,
        })
    })();
    finish(command, output_json, result, |result| {
        format!(
            "claude-cli: current profile {}",
            result.profile.as_deref().unwrap_or("(none)")
        )
    })
}

/// Refresh the named profiles, or with `due_only` every profile near expiry.
pub fn refresh(names: &[String], due_only: bool, output_json: bool) -> i32 {
    let command = if due_only {
        "auth auto-refresh"
    } else {
        "auth refresh"
    };
    let names = if due_only {
        match store::list_profiles() {
            Ok(names) => names,
            Err(err) => return emit_error(command, output_json, err),
        }
    } else {
        names.to_vec()
    };
    if names.is_empty() && !due_only {
        return emit_error(
            command,
            output_json,
            AuthError::usage("missing-profile", "name at least one profile to refresh"),
        );
    }

    let _lock = match store::lock_store() {
        Ok(lock) => lock,
        Err(err) => return emit_error(command, output_json, err),
    };
    let margin_ms = shared_env::env_non_empty(REFRESH_MARGIN_ENV)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_REFRESH_MARGIN_SECONDS)
        .saturating_mul(1000);
    let current = store::read_current().ok().flatten();
    let mut result = RefreshResult::default();
    for name in names {
        let outcome = store::read_profile(&name).and_then(|profile| {
            if due_only
                && profile
                    .expires_at_ms()
                    .is_some_and(|expires| expires - now_ms() > margin_ms)
            {
                return Ok(false);
            }
            let refreshed = exchange_refresh_token(&name, &profile)?;
            if let Err(mut err) = store::write_profile(&name, &refreshed) {
                if let Some(path) = store::quarantine(&name, &refreshed.oauth, &refreshed.account) {
                    err.message = format!(
                        "{}; the rotated login was kept in {}",
                        err.message,
                        path.display()
                    );
                }
                return Err(err);
            }
            if current.as_deref() == Some(name.as_str()) {
                store::write_active_access_only(
                    &refreshed.oauth,
                    &refreshed.account,
                    keychain::Mode::Auto,
                )?;
            }
            Ok(true)
        });
        match outcome {
            Ok(true) => result.refreshed.push(name),
            Ok(false) => result.skipped.push(name),
            Err(err) => result.failed.push(RefreshFailure {
                profile: name,
                code: err.code,
                message: err.message,
            }),
        }
    }

    let code = if result.failed.is_empty() {
        exit::SUCCESS
    } else {
        exit::RUNTIME
    };
    if output_json {
        if diag_output::emit_success_result(AUTH_SCHEMA_VERSION, command, &result).is_err() {
            return exit::RUNTIME;
        }
    } else {
        println!(
            "claude-cli: refreshed={} skipped={} failed={}",
            result.refreshed.join(","),
            result.skipped.join(","),
            result
                .failed
                .iter()
                .map(|failure| format!("{}({})", failure.profile, failure.code))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    code
}

/// Exchange a profile's refresh token through Claude Code's documented
/// `CLAUDE_CODE_OAUTH_REFRESH_TOKEN` login path, in an isolated config dir.
fn exchange_refresh_token(name: &str, profile: &Profile) -> AuthResult<Profile> {
    if keychain::enabled() {
        // Claude Code would store the exchanged login in a Keychain item named
        // after the temporary config dir, not in its credentials file.
        return Err(AuthError::runtime(
            "refresh-unsupported-on-keychain-host",
            "authority refresh needs file credential storage; run it on a Linux authority",
        ));
    }
    let refresh_token = profile.refresh_token().ok_or_else(|| {
        AuthError::data(
            "profile-without-refresh-token",
            format!("profile '{name}' has no refresh token"),
        )
    })?;
    let scopes = profile.scopes();
    if scopes.is_empty() {
        return Err(AuthError::data(
            "profile-without-scopes",
            format!("profile '{name}' does not record its OAuth scopes"),
        ));
    }
    let workdir = tempfile::Builder::new()
        .prefix("claude-auth-refresh-")
        .tempdir()
        .map_err(|err| AuthError::runtime("refresh-workdir-failed", err.to_string()))?;

    let mut command = Command::new(claude_binary());
    command
        .args(["auth", "login"])
        .env(store::CONFIG_DIR_ENV, workdir.path())
        .env("CLAUDE_CODE_OAUTH_REFRESH_TOKEN", refresh_token)
        .env("CLAUDE_CODE_OAUTH_SCOPES", scopes.join(" "))
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .stdin(Stdio::null());
    let output =
        output_with_limits_retry_io(&mut command, REFRESH_TIMEOUT, MAX_REFRESH_OUTPUT_BYTES, 1)
            .map_err(|err| match err {
                ProcessOutputError::Io(err) => {
                    AuthError::runtime("refresh-launch-failed", format!("claude: {err}"))
                }
                ProcessOutputError::Timeout => AuthError::runtime(
                    "refresh-timeout",
                    "claude auth login did not finish in time",
                ),
                ProcessOutputError::OutputLimit => AuthError::runtime(
                    "refresh-output-too-large",
                    "claude auth login output too large",
                ),
            })?;
    if !output.status.success() {
        return Err(AuthError::runtime(
            "refresh-rejected",
            format!(
                "claude auth login exited {}",
                output.status.code().unwrap_or(-1)
            ),
        ));
    }

    let mut credentials = store::read_json_object(
        &workdir.path().join(".credentials.json"),
        "refresh-output-invalid",
    )?
    .unwrap_or_default();
    let mut oauth = store::take_object(&mut credentials, "claudeAiOauth");
    if store::non_empty_str(oauth.get("accessToken")).is_none() {
        return Err(AuthError::runtime(
            "refresh-output-invalid",
            "claude auth login stored no access token",
        ));
    }
    if store::non_empty_str(oauth.get("refreshToken")).is_none() {
        oauth.insert(
            "refreshToken".to_string(),
            Value::String(refresh_token.to_string()),
        );
    }
    if !oauth.contains_key("scopes") {
        oauth.insert(
            "scopes".to_string(),
            Value::Array(scopes.into_iter().map(Value::String).collect()),
        );
    }

    // From here the exchange has issued a new login; a later failure keeps it
    // in quarantine instead of dropping a token the server already rotated.
    let account = (|| -> AuthResult<Map<String, Value>> {
        let refreshed_account = store::read_json_object(
            &workdir.path().join(".claude.json"),
            "refresh-output-invalid",
        )?
        .map(|mut config| store::take_object(&mut config, "oauthAccount"))
        .unwrap_or_default();
        let mut account = profile.account.clone();
        if !refreshed_account.is_empty() {
            let same = match (
                Identity::from_account(&refreshed_account),
                profile.identity(),
            ) {
                (Some(refreshed), Some(stored)) => refreshed.same_account(&stored),
                _ => false,
            };
            if !same {
                return Err(AuthError::data(
                    "refresh-identity-mismatch",
                    format!("the refreshed login for '{name}' belongs to a different account"),
                ));
            }
            merge(&mut account, refreshed_account);
        }
        Ok(account)
    })();
    match account {
        Ok(account) => Ok(Profile { oauth, account }),
        Err(mut err) => {
            if let Some(path) = store::quarantine(name, &oauth, &profile.account) {
                err.message = format!(
                    "{}; the rotated login was kept in {}",
                    err.message,
                    path.display()
                );
            }
            Err(err)
        }
    }
}

fn merge(target: &mut Map<String, Value>, source: Map<String, Value>) {
    for (key, value) in source {
        target.insert(key, value);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn finish<T: Serialize>(
    command: &str,
    output_json: bool,
    result: AuthResult<T>,
    text: impl FnOnce(&T) -> String,
) -> i32 {
    match result {
        Ok(result) => {
            if output_json {
                if diag_output::emit_success_result(AUTH_SCHEMA_VERSION, command, &result).is_err()
                {
                    return exit::RUNTIME;
                }
            } else {
                println!("{}", text(&result));
            }
            exit::SUCCESS
        }
        Err(err) => emit_error(command, output_json, err),
    }
}

pub(crate) fn emit_error(command: &str, output_json: bool, err: AuthError) -> i32 {
    if output_json {
        let _ = diag_output::emit_error(
            AUTH_SCHEMA_VERSION,
            command,
            err.code,
            err.message,
            err.details,
        );
    } else if err.message.starts_with("claude-remote-") {
        // Shared transport messages already carry their own prefix.
        eprintln!("{}", err.message);
    } else {
        eprintln!("claude-cli {command}: {}", err.message);
    }
    err.exit_code
}
