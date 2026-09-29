pub mod auto_refresh;
pub mod current;
pub mod login;
pub mod output;
pub mod refresh;
pub mod remote;
pub mod remove;
pub mod save;
pub mod status;
pub mod sync;
pub mod use_secret;

use anyhow::Result;
use nils_common::provider_runtime::accounts;
use std::path::Path;

pub const ACCESS_ONLY_REFRESH_TOKEN_PLACEHOLDER: &str = "codex-remote-access-only-placeholder";

pub fn identity_from_auth_file(path: &Path) -> Result<Option<String>> {
    crate::runtime::auth::identity_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn email_from_auth_file(path: &Path) -> Result<Option<String>> {
    crate::runtime::auth::email_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn account_id_from_auth_file(path: &Path) -> Result<Option<String>> {
    crate::runtime::auth::account_id_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn last_refresh_from_auth_file(path: &Path) -> Result<Option<String>> {
    crate::runtime::auth::last_refresh_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn identity_key_from_auth_file(path: &Path) -> Result<Option<String>> {
    crate::runtime::auth::identity_key_from_auth_file(path).map_err(anyhow::Error::from)
}

pub fn is_invalid_secret_target(target: &str) -> bool {
    accounts::is_invalid_account_target(target)
}

pub fn normalize_secret_file_name(target: &str) -> String {
    accounts::account_file_name(target)
}

pub fn is_real_refresh_token(value: &str) -> bool {
    !value.is_empty() && value != ACCESS_ONLY_REFRESH_TOKEN_PLACEHOLDER
}

#[cfg(test)]
mod tests {
    use super::{is_invalid_secret_target, normalize_secret_file_name};

    #[test]
    fn secret_target_validation_rejects_paths_and_traversal() {
        assert!(is_invalid_secret_target("../a.json"));
        assert!(is_invalid_secret_target("a/b.json"));
        assert!(is_invalid_secret_target(r"a\b.json"));
        assert!(!is_invalid_secret_target("alpha"));
        assert!(!is_invalid_secret_target("alpha.json"));
    }

    #[test]
    fn normalize_secret_file_name_appends_json_suffix_only_once() {
        assert_eq!(normalize_secret_file_name("alpha"), "alpha.json");
        assert_eq!(normalize_secret_file_name("alpha.json"), "alpha.json");
    }
}
