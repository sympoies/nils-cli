//! Resolve account ownership and known token aliases without network or Keychain I/O.
use super::token_key;
use crate::auth::store;
use std::collections::BTreeMap;

pub(super) fn key(token: &str) -> String {
    resolve(token).0
}

/// Return one account key and every known token digest owned by that account.
pub(super) fn resolve(token: &str) -> (String, Vec<String>) {
    let mut known = BTreeMap::new();
    for name in store::list_profiles().unwrap_or_default() {
        if let Ok(profile) = store::read_profile(&name)
            && let Some(access) = store::non_empty_str(profile.oauth.get("accessToken"))
            && let Some(identity) = profile.identity()
        {
            known
                .entry(token_key(access))
                .or_insert_with(|| token_key(&format!("account:{}", identity.account_uuid)));
        }
    }
    if let Some(path) = store::credentials_file()
        && let Ok(Some(credentials)) = store::read_json_object(&path, "active-credentials-invalid")
        && let Some(access) = credentials
            .get("claudeAiOauth")
            .and_then(|oauth| oauth.get("accessToken"))
            .and_then(serde_json::Value::as_str)
        && let Some(config) = store::global_config_file()
        && let Ok(Some(config)) = store::read_json_object(&config, "active-config-invalid")
        && let Some(account) = config
            .get("oauthAccount")
            .and_then(serde_json::Value::as_object)
        && let Some(identity) = store::Identity::from_account(account)
    {
        known
            .entry(token_key(access))
            .or_insert_with(|| token_key(&format!("account:{}", identity.account_uuid)));
    }
    let token_key = token_key(token);
    let account_key = known
        .get(&token_key)
        .cloned()
        .unwrap_or_else(|| token_key.clone());
    let mut aliases: Vec<String> = known
        .into_iter()
        .filter_map(|(token, account)| (account == account_key).then_some(token))
        .collect();
    if !aliases.contains(&token_key) {
        aliases.push(token_key);
    }
    (account_key, aliases)
}
