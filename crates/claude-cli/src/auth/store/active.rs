use super::*;

pub fn write_active_access_only(
    oauth: &Map<String, Value>,
    account: &Map<String, Value>,
    keychain_mode: keychain::Mode,
) -> AuthResult<ActiveWrite> {
    write_active_access_only_with_accounts_lock(oauth, account, keychain_mode, None)
}

/// Write an access-only copy of `oauth` and `account` as the active login.
///
/// Only `claudeAiOauth` is replaced in the credentials file, so other entries
/// such as MCP server tokens are kept. `refreshToken` is set to `""`, which
/// Claude Code treats as having no refresh token, so it never calls the token
/// endpoint. The global config is rewritten only when `oauthAccount` changes.
pub(crate) fn write_active_access_only_with_accounts_lock(
    oauth: &Map<String, Value>,
    account: &Map<String, Value>,
    keychain_mode: keychain::Mode,
    held_accounts: Option<(&Path, &StoreLock)>,
) -> AuthResult<ActiveWrite> {
    let _receipt_lock = crate::auth::writer_receipt::fence_active(held_accounts)?;
    let access = access_only_oauth(oauth);
    let credentials_file = credentials_file()
        .ok_or_else(|| AuthError::runtime("config-dir-unresolved", "cannot resolve ~/.claude"))?;
    write_access_credentials(&credentials_file, &access)?;

    let config_file = global_config_file().ok_or_else(|| {
        AuthError::runtime("config-dir-unresolved", "cannot resolve ~/.claude.json")
    })?;
    let config_updated = write_config_account(&config_file, account, Map::new())?;

    let keychain = keychain::project(&access, keychain_mode)?;
    if let Some(token) = non_empty_str(access.get("accessToken")) {
        crate::prompt_segment::bind_prompt_token(token);
    }
    Ok(ActiveWrite {
        credentials_file,
        config_updated,
        keychain,
    })
}
