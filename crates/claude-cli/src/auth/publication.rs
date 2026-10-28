//! Local publication of an admitted snapshot. No provider or transport operation,
//! roster pruning, authority-profile mutation, or default selection occurs here.
use super::keychain;
use super::profile::emit_error;
use super::store::{self, AuthError, AuthResult};
use super::writer_receipt::Context;
use nils_common::{cli_contract::exit, diag_output};
use serde_json::{Value, json};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

pub fn publish(snapshot: &Path, into: &Path, mode: keychain::Mode, output_json: bool) -> i32 {
    let command = "auth publish";
    let result = (|| -> AuthResult<Value> {
        let invalid = || {
            AuthError::data(
                "publication-snapshot-invalid",
                "publication requires an access-only snapshot with exact generation and attempt context",
            )
        };
        // Bound input before parsing; never echo payloads or private paths.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(snapshot)
            .map_err(|_| invalid())?;
        let metadata = file.metadata().map_err(|_| invalid())?;
        // SAFETY: geteuid has no arguments and does not mutate process state.
        let owner = unsafe { libc::geteuid() };
        if !metadata.is_file() || metadata.uid() != owner || metadata.mode() & 0o777 != 0o600 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let payload: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let context = Context::from_payload(&payload)?.ok_or_else(invalid)?;
        let name = payload
            .get("profile")
            .and_then(Value::as_str)
            .filter(|name| store::is_account_dir_name(name))
            .ok_or_else(invalid)?;
        let oauth = payload
            .get("claudeAiOauth")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        let account = payload
            .get("oauthAccount")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        if store::non_empty_str(oauth.get("accessToken")).is_none()
            || store::Identity::from_account(account).is_none()
        {
            return Err(invalid());
        }
        let into = store::absolute_accounts_dir(into)?;
        let _lock = store::lock_accounts(&into)?;
        let written =
            store::write_account_publication(&into, name, oauth, account, mode, Some(&context))?;
        Ok(
            json!({"writer_receipt":written.receipt, "keychain":written.keychain, "has_refresh_token":false}),
        )
    })();
    match result.map_err(super::writer_receipt::redact_error) {
        Ok(result) => {
            if output_json {
                if diag_output::emit_success_result(super::AUTH_SCHEMA_VERSION, command, &result)
                    .is_err()
                {
                    return exit::RUNTIME;
                }
            } else {
                println!(
                    "claude-cli: account stores verified for the supplied publication attempt"
                );
            }
            exit::SUCCESS
        }
        Err(err) => emit_error(command, output_json, err),
    }
}
