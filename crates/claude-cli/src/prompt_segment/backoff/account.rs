//! Resolve a token to a known account without network or Keychain I/O.
use super::token_key;
use crate::auth::store;

pub(super) fn key(token: &str) -> String {
    let profiles = store::list_profiles().unwrap_or_default();
    for name in profiles {
        if let Ok(profile) = store::read_profile(&name)
            && store::non_empty_str(profile.oauth.get("accessToken")) == Some(token)
            && let Some(identity) = profile.identity()
        {
            return token_key(&format!("account:{}", identity.account_uuid));
        }
    }
    if let Some(path) = store::credentials_file()
        && let Ok(Some(object)) = store::read_json_object(&path, "active-credentials-invalid")
        && object
            .get("claudeAiOauth")
            .and_then(|oauth| oauth.get("accessToken"))
            .and_then(serde_json::Value::as_str)
            == Some(token)
        && let Some(config) = store::global_config_file()
        && let Ok(Some(object)) = store::read_json_object(&config, "active-config-invalid")
        && let Some(account) = object
            .get("oauthAccount")
            .and_then(serde_json::Value::as_object)
        && let Some(identity) = store::Identity::from_account(account)
    {
        return token_key(&format!("account:{}", identity.account_uuid));
    }
    token_key(token)
}
