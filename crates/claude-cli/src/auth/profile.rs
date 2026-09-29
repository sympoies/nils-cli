//! Authority-side profile commands: save, use, current, refresh, auto-refresh.

use nils_common::cli_contract::exit;
use nils_common::diag_output;
use nils_common::env as shared_env;
use nils_common::provider_runtime::accounts::{
    self, AccountResolution, ConfirmAction, Confirmation,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::AUTH_SCHEMA_VERSION;
use super::keychain;
use super::store::{self, AuthError, AuthResult, Identity, Profile};
use crate::agent::oneshot::claude_binary;
use crate::process::{ProcessOutputError, output_with_limits_retry_io};

const REFRESH_MARGIN_ENV: &str = "CLAUDE_AUTH_REFRESH_MARGIN_SECONDS";
/// Accounts directory refreshed profiles are re-projected into.
pub const ACCOUNTS_DIR_ENV: &str = "CLAUDE_ACCOUNTS_DIR";
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
    target: String,
    profile: String,
    account_uuid: Option<String>,
    credentials_file: String,
    config_updated: bool,
    keychain: &'static str,
}

#[derive(Serialize)]
struct RemoveResult {
    profile: String,
    removed: bool,
}

#[derive(Serialize)]
struct CurrentResult {
    matched: bool,
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
    /// Refreshed profiles re-projected into the accounts directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    projected: Option<Vec<String>>,
    /// Refreshed profiles whose account projection failed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    projection_failed: Vec<RefreshFailure>,
}

/// The refresh-capable active login to save as `name`, and whether it would
/// replace an existing profile of the same account.
fn saveable_login(name: &str) -> AuthResult<(Profile, Identity, bool)> {
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
    Ok((login, identity, replaced))
}

pub fn save(target: &str, yes: bool, output_json: bool) -> i32 {
    let command = "auth save";
    let result = (|| -> AuthResult<SaveResult> {
        let name = store::profile_name_from_target(target)?;
        // Ask before taking the store lock so a pending prompt never blocks a
        // concurrent refresh; the checks run again under the lock.
        let (_, _, replaces) = saveable_login(&name)?;
        if replaces {
            confirm(
                ConfirmAction::Overwrite,
                yes,
                output_json,
                &format!("claude-cli auth save: profile '{name}' exists. overwrite?"),
                &name,
            )?;
        }
        let _lock = store::lock_store()?;
        let (login, identity, replaced) = saveable_login(&name)?;
        overwrite_allowed_under_lock(&name, replaces, replaced, yes)?;
        store::write_profile(&name, &login)?;
        // The profile is now the only refresher: leave the source login
        // access-only. On a Keychain host the Keychain copy is what Claude Code
        // reads first, so failing to rewrite it must fail the save.
        let mode = if keychain::enabled() {
            keychain::Mode::Required
        } else {
            keychain::Mode::Auto
        };
        store::write_active_access_only(&login.oauth, &login.account, mode)?;
        Ok(SaveResult {
            profile: name,
            account_uuid: identity.account_uuid,
            replaced,
        })
    })();
    finish(command, output_json, result, |result| {
        format!("claude-cli: saved profile '{}'", result.profile)
    })
}

/// The profile `target` names: a profile name, `name.json`, a full email
/// address, or an email local part.
fn resolve_profile(target: &str) -> AuthResult<String> {
    if !target.contains('@') || accounts::is_invalid_account_target(target) {
        store::profile_name_from_target(target)?;
    }
    let details = |candidates: Option<Vec<String>>| {
        let mut details = json!({ "target": target });
        if let Some(candidates) = candidates {
            details["candidates"] = json!(candidates);
        }
        Some(details)
    };
    match accounts::resolve_account(&store::ProfileStore, target) {
        AccountResolution::Exact(file_name) => Ok(accounts::account_name(&file_name).to_string()),
        AccountResolution::Ambiguous { candidates } => {
            let candidates: Vec<String> = candidates
                .iter()
                .map(|file_name| accounts::account_name(file_name).to_string())
                .collect();
            Err(AuthError {
                code: "ambiguous-profile",
                message: format!(
                    "'{target}' matches several profiles: {}",
                    candidates.join(", ")
                ),
                exit_code: accounts::EXIT_UNMATCHED,
                details: details(Some(candidates)),
            })
        }
        AccountResolution::NotFound => Err(AuthError {
            code: "profile-not-found",
            message: format!("no profile matches '{target}'"),
            exit_code: accounts::EXIT_FAILED,
            details: details(None),
        }),
    }
}

pub fn use_profile(target: &str, output_json: bool) -> i32 {
    let command = "auth use";
    let result = (|| -> AuthResult<UseResult> {
        let _lock = store::lock_store()?;
        let name = resolve_profile(target)?;
        let profile = store::read_profile(&name)?;
        let written = store::write_active_access_only(
            &profile.oauth,
            &profile.account,
            keychain::Mode::Auto,
        )?;
        store::write_current(&name)?;
        Ok(UseResult {
            target: target.to_string(),
            profile: name,
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

/// Refuse removing a missing profile or the current default.
fn removable_profile(name: &str) -> AuthResult<()> {
    if !store::profile_file(name)?.is_file() {
        return Err(AuthError::runtime(
            "profile-not-found",
            format!("profile '{name}' does not exist"),
        ));
    }
    if store::read_current()?.as_deref() == Some(name) {
        return Err(AuthError::runtime(
            "profile-is-current-default",
            format!(
                "profile '{name}' is the current default; switch with `claude-cli auth use <name>` first"
            ),
        ));
    }
    Ok(())
}

pub fn remove(target: &str, yes: bool, output_json: bool) -> i32 {
    let command = "auth remove";
    let result = (|| -> AuthResult<RemoveResult> {
        let name = store::profile_name_from_target(target)?;
        removable_profile(&name)?;
        confirm(
            ConfirmAction::Remove,
            yes,
            output_json,
            &format!("claude-cli auth remove: remove profile '{name}'?"),
            &name,
        )?;
        let _lock = store::lock_store()?;
        removable_profile(&name)?;
        store::remove_profile(&name)?;
        Ok(RemoveResult {
            profile: name,
            removed: true,
        })
    })();
    finish(command, output_json, result, |result| {
        format!("claude-cli: removed profile '{}'", result.profile)
    })
}

/// Whether the locked save may write profile `name`: a profile that appeared
/// after the unlocked check was never confirmed, so it needs `--yes`.
fn overwrite_allowed_under_lock(
    name: &str,
    confirmed: bool,
    replaced: bool,
    yes: bool,
) -> AuthResult<()> {
    if replaced && !confirmed && !yes {
        return Err(confirmation_required(ConfirmAction::Overwrite, name));
    }
    Ok(())
}

fn confirm_verb(action: ConfirmAction) -> &'static str {
    match action {
        ConfirmAction::Overwrite => "overwrite",
        ConfirmAction::Remove => "remove",
    }
}

fn confirmation_required(action: ConfirmAction, name: &str) -> AuthError {
    AuthError {
        code: action.required_error_code(),
        message: format!(
            "profile '{name}' exists; rerun with --yes to {} it",
            confirm_verb(action)
        ),
        exit_code: action.required_exit_code(),
        details: Some(json!({ "profile": name })),
    }
}

/// Run the shared confirmation flow for `action` on profile `name`.
fn confirm(
    action: ConfirmAction,
    yes: bool,
    output_json: bool,
    prompt: &str,
    name: &str,
) -> AuthResult<()> {
    let verb = confirm_verb(action);
    let answer = accounts::confirm(yes, output_json, prompt)
        .map_err(|err| AuthError::runtime("confirmation-failed", err.to_string()))?;
    match answer {
        Confirmation::Confirmed => Ok(()),
        Confirmation::Required => Err(confirmation_required(action, name)),
        Confirmation::Declined => Err(AuthError {
            code: "confirmation-declined",
            message: format!("{verb} declined for profile '{name}'"),
            exit_code: action.declined_exit_code(),
            details: Some(json!({ "profile": name })),
        }),
    }
}

pub fn current(output_json: bool) -> i32 {
    let command = "auth current";
    let result = (|| -> AuthResult<CurrentResult> {
        let profiles = store::list_profiles()?;
        let Some(name) = store::read_current()? else {
            return Ok(CurrentResult {
                matched: false,
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
            matched: true,
            profile: Some(name),
            account_uuid: identity.as_ref().map(|id| id.account_uuid.clone()),
            organization_uuid: identity.map(|id| id.organization_uuid),
            expires_at: profile.expires_at_ms(),
            profiles,
        })
    })();
    let matched = result.as_ref().is_ok_and(|result| result.matched);
    let code = finish(command, output_json, result, |result| {
        format!(
            "claude-cli: current profile {}",
            result.profile.as_deref().unwrap_or("(none)")
        )
    });
    if code == exit::SUCCESS && !matched {
        accounts::EXIT_UNMATCHED
    } else {
        code
    }
}

/// Refresh the named profiles, or with `due_only` every profile near expiry.
///
/// With an accounts directory (`accounts_dir`, else `CLAUDE_ACCOUNTS_DIR`),
/// each refreshed profile is also re-projected access-only into
/// `<accounts dir>/<profile>/`, file storage only.
pub fn refresh(
    names: &[String],
    due_only: bool,
    accounts_dir: Option<&Path>,
    output_json: bool,
) -> i32 {
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

    let accounts_dir = match accounts_dir
        .map(Path::to_path_buf)
        .or_else(|| shared_env::env_non_empty(ACCOUNTS_DIR_ENV).map(Into::into))
        .map(|dir| store::absolute_accounts_dir(&dir))
        .transpose()
    {
        Ok(dir) => dir,
        Err(err) => return emit_error(command, output_json, err),
    };

    let _lock = match store::lock_store() {
        Ok(lock) => lock,
        Err(err) => return emit_error(command, output_json, err),
    };
    let _accounts_lock = match accounts_dir
        .as_deref()
        .map(store::lock_accounts)
        .transpose()
    {
        Ok(lock) => lock,
        Err(err) => return emit_error(command, output_json, err),
    };
    let margin_ms = shared_env::env_non_empty(REFRESH_MARGIN_ENV)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_REFRESH_MARGIN_SECONDS)
        .saturating_mul(1000);
    let current = store::read_current().ok().flatten();
    let mut result = RefreshResult {
        projected: accounts_dir.as_ref().map(|_| Vec::new()),
        ..RefreshResult::default()
    };
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
            if let Some(dir) = accounts_dir.as_deref() {
                // Authority refresh already refuses Keychain hosts: file only.
                let projection = store::write_account_access_only(
                    dir,
                    &name,
                    &refreshed.oauth,
                    &refreshed.account,
                    keychain::Mode::Off,
                );
                match projection {
                    Ok(_) => result
                        .projected
                        .get_or_insert_with(Vec::new)
                        .push(name.clone()),
                    Err(err) => result.projection_failed.push(RefreshFailure {
                        profile: name.clone(),
                        code: err.code,
                        message: err.message,
                    }),
                }
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

    let code = if result.failed.is_empty() && result.projection_failed.is_empty() {
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
        if let Some(projected) = &result.projected {
            println!(
                "claude-cli: projected={} projection_failed={}",
                projected.join(","),
                result
                    .projection_failed
                    .iter()
                    .map(|failure| format!("{}({})", failure.profile, failure.code))
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn locked_save_refuses_a_profile_that_appeared_after_the_unlocked_check() {
        let err = overwrite_allowed_under_lock("team", false, true, false)
            .expect_err("a profile created after the pre-lock check needs confirmation");
        assert_eq!(err.code, "overwrite-confirmation-required");
        assert_eq!(err.exit_code, 1);
        assert_eq!(err.details, Some(json!({ "profile": "team" })));
    }

    #[test]
    fn locked_save_allows_confirmed_yes_and_new_profiles() {
        // Confirmed before the lock, --yes given, or still a new profile.
        for (confirmed, replaced, yes) in [
            (true, true, false),
            (false, true, true),
            (false, false, false),
            (true, false, false),
        ] {
            assert!(
                overwrite_allowed_under_lock("team", confirmed, replaced, yes).is_ok(),
                "confirmed={confirmed} replaced={replaced} yes={yes}"
            );
        }
    }
}
