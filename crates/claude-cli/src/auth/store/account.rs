use super::*;
use crate::auth::writer_receipt::{Context, Receipt, Store, invalidate, store_step, verify};

pub fn write_account_access_only(
    accounts_dir: &Path,
    name: &str,
    oauth: &Map<String, Value>,
    account: &Map<String, Value>,
    keychain_mode: keychain::Mode,
) -> AuthResult<AccountWrite> {
    write_account_publication(accounts_dir, name, oauth, account, keychain_mode, None)
}

/// Write an access-only copy of a profile into `<accounts_dir>/<name>/`, a
/// Claude Code config dir of its own.
///
/// The directory gets the ownership marker, a real `.credentials.json` with
/// `refreshToken: ""`, and `oauthAccount` in its `.claude.json` (seeded with
/// `hasCompletedOnboarding` when absent). With a Keychain the item named after
/// that config dir is written as well. `accounts_dir` must be absolute, since
/// the Keychain item name is derived from the path.
pub(crate) fn write_account_publication(
    accounts_dir: &Path,
    name: &str,
    oauth: &Map<String, Value>,
    account: &Map<String, Value>,
    keychain_mode: keychain::Mode,
    context: Option<&Context>,
) -> AuthResult<AccountWrite> {
    if !is_account_dir_name(name) {
        return Err(AuthError::data(
            "account-dir-invalid",
            format!("'{name}' cannot name an account directory"),
        ));
    }
    let config_dir = accounts_dir.join(name);
    create_private_dir(accounts_dir, "account-dir-write-failed")?;
    match std::fs::symlink_metadata(&config_dir) {
        Ok(meta) if !meta.is_dir() => {
            return Err(AuthError::data(
                "account-dir-invalid",
                format!("{} is not a directory", config_dir.display()),
            ));
        }
        Ok(_) => {
            // Adopt only a directory claude-cli created, or an empty one.
            let owned = std::fs::symlink_metadata(config_dir.join(ACCOUNT_MARKER))
                .is_ok_and(|meta| meta.is_file());
            let empty = std::fs::read_dir(&config_dir)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false);
            if !owned && !empty {
                return Err(AuthError::data(
                    "account-dir-not-owned",
                    format!(
                        "{} exists without the claude-cli ownership marker",
                        config_dir.display()
                    ),
                ));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            create_private_dir(&config_dir, "account-dir-write-failed")?;
        }
        Err(err) => {
            return Err(AuthError::runtime(
                "account-dir-write-failed",
                format!("cannot inspect {}: {err}", config_dir.display()),
            ));
        }
    }
    // Fence the previous attempt durably before any store or marker mutation.
    let mut receipt = match context {
        Some(context) => Some(Receipt::begin(&config_dir, context)?),
        None => {
            invalidate(&config_dir)?;
            None
        }
    };
    fs::write_atomic(
        &config_dir.join(ACCOUNT_MARKER),
        format!("{name}\n").as_bytes(),
        fs::SECRET_FILE_MODE,
    )
    .map_err(|err| AuthError::runtime("account-dir-write-failed", err.to_string()))?;

    let access = access_only_oauth(oauth);
    let mut seed = Map::new();
    seed.insert("hasCompletedOnboarding".to_string(), Value::Bool(true));
    let keychain = (|| -> AuthResult<_> {
        store_step(&config_dir, &mut receipt, Store::Credentials, || {
            write_access_credentials(&config_dir.join(".credentials.json"), &access)?;
            verify(&config_dir, Store::Credentials, &access)
        })?;
        store_step(&config_dir, &mut receipt, Store::Config, || {
            write_config_account(&config_dir.join(".claude.json"), account, seed)?;
            verify(&config_dir, Store::Config, account)
        })?;
        let keychain = keychain::project_for_config_dir(
            &config_dir.to_string_lossy(),
            &access,
            keychain_mode,
        )?;
        // Re-read after projection as well: an external writer cannot turn a
        // changed store into successful evidence during this publication.
        if receipt.is_some() {
            store_step(&config_dir, &mut receipt, Store::Credentials, || {
                verify(&config_dir, Store::Credentials, &access)
            })?;
            store_step(&config_dir, &mut receipt, Store::Config, || {
                verify(&config_dir, Store::Config, account)
            })?;
        }
        Ok(keychain)
    })()?;
    if let Some(receipt) = receipt.as_mut() {
        receipt.complete(&config_dir)?;
    }
    Ok(AccountWrite {
        config_dir,
        keychain,
        receipt,
    })
}
