pub mod auto_refresh;
pub mod current;
pub mod login;
pub mod output;
pub mod refresh;
pub mod remove;
pub mod save;
pub mod sync;
pub mod use_secret;

use anyhow::Result;
use std::path::Path;

pub fn identity_from_auth_file(path: &Path) -> Result<Option<String>> {
    codex_core::auth::identity_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn email_from_auth_file(path: &Path) -> Result<Option<String>> {
    codex_core::auth::email_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn account_id_from_auth_file(path: &Path) -> Result<Option<String>> {
    codex_core::auth::account_id_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn last_refresh_from_auth_file(path: &Path) -> Result<Option<String>> {
    codex_core::auth::last_refresh_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn identity_key_from_auth_file(path: &Path) -> Result<Option<String>> {
    codex_core::auth::identity_key_from_auth_file(path).map_err(anyhow::Error::from)
}
