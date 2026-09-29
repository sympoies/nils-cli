//! Claude Code login storage: the active login Claude Code reads, and the
//! named authority profiles that hold refresh-capable copies.
//!
//! Profiles live in `CLAUDE_SECRET_DIR` (default `~/.config/claude_secrets`)
//! as `<name>.json` holding `{"claudeAiOauth": ..., "oauthAccount": ...}`.
//! `<secret dir>/current` names the authority's current default profile.

use nils_common::cli_contract::exit;
use nils_common::env as shared_env;
use nils_common::fs;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use super::keychain;

pub const SECRET_DIR_ENV: &str = "CLAUDE_SECRET_DIR";
pub const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const CURRENT_FILE: &str = "current";
const PRIVATE_DIR_MODE: u32 = 0o700;

/// The `claudeAiOauth` fields an access-only replica may hold.
const ACCESS_FIELDS: &[&str] = &[
    "accessToken",
    "expiresAt",
    "scopes",
    "subscriptionType",
    "rateLimitTier",
];

#[derive(Debug)]
pub struct AuthError {
    pub code: &'static str,
    pub message: String,
    pub exit_code: i32,
}

impl AuthError {
    pub fn data(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            exit_code: exit::DATA,
        }
    }

    pub fn runtime(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            exit_code: exit::RUNTIME,
        }
    }

    pub fn usage(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            exit_code: exit::USAGE,
        }
    }
}

pub type AuthResult<T> = Result<T, AuthError>;

/// Claude Code's config directory: `CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn config_dir() -> Option<PathBuf> {
    match shared_env::env_non_empty(CONFIG_DIR_ENV) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(fs::home_dir()?.join(".claude")),
    }
}

pub fn credentials_file() -> Option<PathBuf> {
    Some(config_dir()?.join(".credentials.json"))
}

/// Claude Code's global config: `$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`.
pub fn global_config_file() -> Option<PathBuf> {
    match shared_env::env_non_empty(CONFIG_DIR_ENV) {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => Some(fs::home_dir()?.join(".claude.json")),
    }
}

pub fn secret_dir() -> Option<PathBuf> {
    match shared_env::env_non_empty(SECRET_DIR_ENV) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(fs::home_dir()?.join(".config").join("claude_secrets")),
    }
}

fn required_secret_dir() -> AuthResult<PathBuf> {
    secret_dir().ok_or_else(|| {
        AuthError::runtime("secret-dir-unresolved", "cannot resolve CLAUDE_SECRET_DIR")
    })
}

pub fn validate_profile_name(name: &str) -> AuthResult<()> {
    if nils_common::provider_runtime::remote::is_valid_secret_name(name) && !name.ends_with(".json")
    {
        Ok(())
    } else {
        Err(AuthError::usage(
            "invalid-profile-name",
            "profile names use [A-Za-z0-9._-] and no .json suffix",
        ))
    }
}

pub fn profile_file(name: &str) -> AuthResult<PathBuf> {
    validate_profile_name(name)?;
    Ok(required_secret_dir()?.join(format!("{name}.json")))
}

pub fn read_json_object(path: &Path, code: &'static str) -> AuthResult<Option<Map<String, Value>>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(AuthError::runtime(
                code,
                format!("cannot read {}: {err}", path.display()),
            ));
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        _ => Err(AuthError::data(
            code,
            format!("{} is not a JSON object", path.display()),
        )),
    }
}

fn write_json(path: &Path, value: &Value, mode: u32, code: &'static str) -> AuthResult<()> {
    if let Some(parent) = path.parent() {
        create_private_dir(parent, code)?;
    }
    let bytes = serde_json::to_vec(value)
        .map_err(|err| AuthError::runtime(code, format!("cannot encode JSON: {err}")))?;
    fs::write_atomic(path, &bytes, mode)
        .map_err(|err| AuthError::runtime(code, format!("cannot write {}: {err}", path.display())))
}

fn create_private_dir(dir: &Path, code: &'static str) -> AuthResult<()> {
    if dir.is_dir() {
        return Ok(());
    }
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(dir)
        .map_err(|err| AuthError::runtime(code, format!("cannot create {}: {err}", dir.display())))
}

fn existing_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

/// A stored authority profile.
#[derive(Debug, Clone)]
pub struct Profile {
    pub oauth: Map<String, Value>,
    pub account: Map<String, Value>,
}

impl Profile {
    pub fn identity(&self) -> Option<Identity> {
        Identity::from_account(&self.account)
    }

    pub fn refresh_token(&self) -> Option<&str> {
        non_empty_str(self.oauth.get("refreshToken"))
    }

    pub fn expires_at_ms(&self) -> Option<i64> {
        self.oauth.get("expiresAt").and_then(Value::as_i64)
    }

    pub fn scopes(&self) -> Vec<String> {
        self.oauth
            .get("scopes")
            .and_then(Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert(
            "claudeAiOauth".to_string(),
            Value::Object(self.oauth.clone()),
        );
        object.insert(
            "oauthAccount".to_string(),
            Value::Object(self.account.clone()),
        );
        Value::Object(object)
    }
}

pub fn read_profile(name: &str) -> AuthResult<Profile> {
    let path = profile_file(name)?;
    let Some(mut object) = read_json_object(&path, "profile-invalid")? else {
        return Err(AuthError::data(
            "profile-not-found",
            format!("profile '{name}' does not exist"),
        ));
    };
    let oauth = take_object(&mut object, "claudeAiOauth");
    let account = take_object(&mut object, "oauthAccount");
    if non_empty_str(oauth.get("accessToken")).is_none() {
        return Err(AuthError::data(
            "profile-invalid",
            format!("profile '{name}' has no access token"),
        ));
    }
    Ok(Profile { oauth, account })
}

pub fn write_profile(name: &str, profile: &Profile) -> AuthResult<()> {
    let path = profile_file(name)?;
    write_json(
        &path,
        &profile.to_value(),
        fs::SECRET_FILE_MODE,
        "profile-write-failed",
    )
}

pub fn list_profiles() -> AuthResult<Vec<String>> {
    let dir = required_secret_dir()?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(AuthError::runtime(
                "secret-dir-unreadable",
                format!("cannot read {}: {err}", dir.display()),
            ));
        }
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().into_string().ok()?;
            let name = file_name.strip_suffix(".json")?.to_string();
            validate_profile_name(&name).ok().map(|()| name)
        })
        .collect();
    names.sort();
    Ok(names)
}

pub fn read_current() -> AuthResult<Option<String>> {
    let path = required_secret_dir()?.join(CURRENT_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let name = text.trim().to_string();
            validate_profile_name(&name)?;
            Ok(Some(name))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(AuthError::runtime(
            "current-unreadable",
            format!("cannot read {}: {err}", path.display()),
        )),
    }
}

pub fn write_current(name: &str) -> AuthResult<()> {
    validate_profile_name(name)?;
    let dir = required_secret_dir()?;
    create_private_dir(&dir, "current-write-failed")?;
    fs::write_atomic(
        &dir.join(CURRENT_FILE),
        format!("{name}\n").as_bytes(),
        fs::SECRET_FILE_MODE,
    )
    .map_err(|err| AuthError::runtime("current-write-failed", err.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub account_uuid: String,
    pub organization_uuid: String,
}

impl Identity {
    pub fn from_account(account: &Map<String, Value>) -> Option<Self> {
        Some(Self {
            account_uuid: non_empty_str(account.get("accountUuid"))?.to_string(),
            organization_uuid: non_empty_str(account.get("organizationUuid"))
                .unwrap_or_default()
                .to_string(),
        })
    }
}

/// The active login Claude Code reads, as a refresh-capable profile.
pub fn read_active_login() -> AuthResult<Profile> {
    let credentials = credentials_file()
        .ok_or_else(|| AuthError::runtime("config-dir-unresolved", "cannot resolve ~/.claude"))?;
    let mut object = read_json_object(&credentials, "active-credentials-invalid")?;
    if object
        .as_ref()
        .is_none_or(|object| !object.contains_key("claudeAiOauth"))
        && keychain::enabled()
    {
        object = keychain::read_item()?;
    }
    let oauth = object
        .as_mut()
        .map(|object| take_object(object, "claudeAiOauth"))
        .unwrap_or_default();
    if non_empty_str(oauth.get("accessToken")).is_none() {
        return Err(AuthError::data(
            "active-login-missing",
            "no Claude Code login is stored for this config dir",
        ));
    }
    let config = global_config_file().ok_or_else(|| {
        AuthError::runtime("config-dir-unresolved", "cannot resolve ~/.claude.json")
    })?;
    let account = read_json_object(&config, "active-config-invalid")?
        .map(|mut config| take_object(&mut config, "oauthAccount"))
        .unwrap_or_default();
    Ok(Profile { oauth, account })
}

/// Result of projecting a login into the active Claude Code storage.
#[derive(Debug)]
pub struct ActiveWrite {
    pub credentials_file: PathBuf,
    pub config_updated: bool,
    pub keychain: &'static str,
}

/// Write an access-only copy of `oauth` and `account` as the active login.
///
/// Only `claudeAiOauth` is replaced in the credentials file, so other entries
/// such as MCP server tokens are kept. `refreshToken` is set to `""`, which
/// Claude Code treats as having no refresh token, so it never calls the token
/// endpoint. The global config is rewritten only when `oauthAccount` changes.
pub fn write_active_access_only(
    oauth: &Map<String, Value>,
    account: &Map<String, Value>,
    keychain_mode: keychain::Mode,
) -> AuthResult<ActiveWrite> {
    let access = access_only_oauth(oauth);
    let credentials_file = credentials_file()
        .ok_or_else(|| AuthError::runtime("config-dir-unresolved", "cannot resolve ~/.claude"))?;
    let mut credentials =
        read_json_object(&credentials_file, "active-credentials-invalid")?.unwrap_or_default();
    credentials.insert("claudeAiOauth".to_string(), Value::Object(access.clone()));
    let credentials = Value::Object(credentials);
    write_json(
        &credentials_file,
        &credentials,
        fs::SECRET_FILE_MODE,
        "active-credentials-write-failed",
    )?;

    let config_file = global_config_file().ok_or_else(|| {
        AuthError::runtime("config-dir-unresolved", "cannot resolve ~/.claude.json")
    })?;
    let mut config = read_json_object(&config_file, "active-config-invalid")?.unwrap_or_default();
    let config_updated = config.get("oauthAccount") != Some(&Value::Object(account.clone()));
    if config_updated {
        let mode = existing_mode(&config_file).unwrap_or(fs::SECRET_FILE_MODE);
        config.insert("oauthAccount".to_string(), Value::Object(account.clone()));
        write_json(
            &config_file,
            &Value::Object(config),
            mode,
            "active-config-write-failed",
        )?;
    }

    let keychain = keychain::project(&access, keychain_mode)?;
    Ok(ActiveWrite {
        credentials_file,
        config_updated,
        keychain,
    })
}

/// The access-only subset of `claudeAiOauth`, with the empty refresh token.
pub fn access_only_oauth(oauth: &Map<String, Value>) -> Map<String, Value> {
    let mut access = access_fields(oauth);
    access.insert("refreshToken".to_string(), Value::String(String::new()));
    access
}

/// The `claudeAiOauth` fields a replica may receive, without any refresh token.
pub fn access_fields(oauth: &Map<String, Value>) -> Map<String, Value> {
    ACCESS_FIELDS
        .iter()
        .filter_map(|key| Some(((*key).to_string(), oauth.get(*key)?.clone())))
        .collect()
}

pub fn take_object(object: &mut Map<String, Value>, key: &str) -> Map<String, Value> {
    match object.remove(key) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

pub fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn access_only_oauth_drops_the_refresh_token_and_unknown_fields() {
        let oauth = json!({
            "accessToken": "a",
            "refreshToken": "r",
            "expiresAt": 5,
            "scopes": ["user:inference"],
            "subscriptionType": "max",
            "rateLimitTier": "tier",
            "future": "x"
        });
        let access = access_only_oauth(oauth.as_object().expect("object"));
        assert_eq!(
            Value::Object(access),
            json!({
                "accessToken": "a",
                "refreshToken": "",
                "expiresAt": 5,
                "scopes": ["user:inference"],
                "subscriptionType": "max",
                "rateLimitTier": "tier"
            })
        );
    }

    #[test]
    fn profile_names_reject_paths_and_json_suffixes() {
        assert!(validate_profile_name("team").is_ok());
        assert!(validate_profile_name("team.json").is_err());
        assert!(validate_profile_name("../team").is_err());
        assert!(validate_profile_name("").is_err());
    }
}
