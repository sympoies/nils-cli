use anyhow::Result;
use nils_common::fs;
use nils_common::provider_runtime::persistence::{
    SyncSecretsError, TimestampPolicy, sync_auth_to_matching_secrets,
};
use serde_json::json;

use crate::auth::output::{self, AuthSyncResult};
use crate::paths;
use crate::provider_profile::CODEX_PROVIDER_PROFILE;

pub fn run() -> Result<i32> {
    run_with_json(false)
}

pub fn run_with_json(output_json: bool) -> Result<i32> {
    let auth_file = match paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            if output_json {
                output::emit_result(
                    "auth sync",
                    AuthSyncResult {
                        auth_file: String::new(),
                        synced: 0,
                        skipped: 0,
                        failed: 0,
                        updated_files: Vec::new(),
                    },
                )?;
            }
            return Ok(0);
        }
    };

    let sync_result = match sync_auth_to_matching_secrets(
        &CODEX_PROVIDER_PROFILE,
        &auth_file,
        fs::SECRET_FILE_MODE,
        TimestampPolicy::Strict,
    ) {
        Ok(result) => result,
        Err(SyncSecretsError::HashAuthFile { path, .. })
        | Err(SyncSecretsError::HashSecretFile { path, .. }) => {
            if output_json {
                output::emit_error(
                    "auth sync",
                    "hash-failed",
                    format!("failed to hash {}", path.display()),
                    Some(json!({
                        "path": path.display().to_string(),
                    })),
                )?;
            } else {
                eprintln!("codex: failed to hash {}", path.display());
            }
            return Ok(1);
        }
        Err(err) => return Err(err.into()),
    };

    if !sync_result.auth_file_present {
        if output_json {
            output::emit_result(
                "auth sync",
                AuthSyncResult {
                    auth_file: auth_file.display().to_string(),
                    synced: 0,
                    skipped: 1,
                    failed: 0,
                    updated_files: Vec::new(),
                },
            )?;
        }
        return Ok(0);
    }

    if !sync_result.auth_identity_present {
        if output_json {
            output::emit_result(
                "auth sync",
                AuthSyncResult {
                    auth_file: auth_file.display().to_string(),
                    synced: 0,
                    skipped: 1,
                    failed: 0,
                    updated_files: Vec::new(),
                },
            )?;
        }
        return Ok(0);
    }

    let synced = sync_result.synced;
    let skipped = sync_result.skipped;
    let failed = 0usize;
    let updated_files = sync_result
        .updated_files
        .into_iter()
        .map(|path| path.display().to_string())
        .collect();

    if output_json {
        output::emit_result(
            "auth sync",
            AuthSyncResult {
                auth_file: auth_file.display().to_string(),
                synced,
                skipped,
                failed,
                updated_files,
            },
        )?;
    }

    Ok(0)
}
