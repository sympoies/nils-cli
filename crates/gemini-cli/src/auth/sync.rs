use nils_common::provider_runtime::persistence::{
    SyncSecretsError, TimestampPolicy, sync_auth_to_matching_secrets,
};

use crate::auth;
use crate::auth::output;
use crate::provider_profile::GEMINI_PROVIDER_PROFILE;

pub fn run() -> i32 {
    run_with_json(false)
}

pub fn run_with_json(output_json: bool) -> i32 {
    let auth_file = match crate::paths::resolve_auth_file() {
        Some(path) => path,
        None => {
            if output_json {
                let _ = output::emit_result(
                    "auth sync",
                    output::obj(vec![
                        ("auth_file", output::s("")),
                        ("synced", output::n(0)),
                        ("skipped", output::n(0)),
                        ("failed", output::n(0)),
                        ("updated_files", output::arr(Vec::new())),
                    ]),
                );
            }
            return 0;
        }
    };

    let sync_result = match sync_auth_to_matching_secrets(
        &GEMINI_PROVIDER_PROFILE,
        &auth_file,
        auth::SECRET_FILE_MODE,
        TimestampPolicy::BestEffort,
    ) {
        Ok(result) => result,
        Err(SyncSecretsError::ReadAuthFile { path, .. })
        | Err(SyncSecretsError::HashAuthFile { path, .. }) => {
            if output_json {
                let _ = output::emit_error(
                    "auth sync",
                    "auth-read-failed",
                    format!("failed to read {}", path.display()),
                    Some(output::obj(vec![(
                        "path",
                        output::s(path.display().to_string()),
                    )])),
                );
            } else {
                eprintln!("gemini: failed to read {}", path.display());
            }
            return 1;
        }
        Err(SyncSecretsError::HashSecretFile { path, .. }) => {
            if output_json {
                let _ = output::emit_error(
                    "auth sync",
                    "secret-read-failed",
                    format!("failed to read {}", path.display()),
                    Some(output::obj(vec![(
                        "path",
                        output::s(path.display().to_string()),
                    )])),
                );
            } else {
                eprintln!("gemini: failed to read {}", path.display());
            }
            return 1;
        }
        Err(SyncSecretsError::WriteSecretFile { path, .. })
        | Err(SyncSecretsError::WriteTimestampFile { path, .. }) => {
            if output_json {
                let _ = output::emit_error(
                    "auth sync",
                    "sync-write-failed",
                    format!("failed to write {}", path.display()),
                    Some(output::obj(vec![(
                        "path",
                        output::s(path.display().to_string()),
                    )])),
                );
            } else {
                eprintln!("gemini: failed to write {}", path.display());
            }
            return 1;
        }
    };

    if !sync_result.auth_file_present {
        if output_json {
            let _ = output::emit_result(
                "auth sync",
                output::obj(vec![
                    ("auth_file", output::s(auth_file.display().to_string())),
                    ("synced", output::n(0)),
                    ("skipped", output::n(1)),
                    ("failed", output::n(0)),
                    ("updated_files", output::arr(Vec::new())),
                ]),
            );
        }
        return 0;
    }

    if !sync_result.auth_identity_present {
        if output_json {
            let _ = output::emit_result(
                "auth sync",
                output::obj(vec![
                    ("auth_file", output::s(auth_file.display().to_string())),
                    ("synced", output::n(0)),
                    ("skipped", output::n(1)),
                    ("failed", output::n(0)),
                    ("updated_files", output::arr(Vec::new())),
                ]),
            );
        }
        return 0;
    }

    let synced = sync_result.synced;
    let skipped = sync_result.skipped;
    let failed = 0usize;
    let updated_files: Vec<String> = sync_result
        .updated_files
        .into_iter()
        .map(|path| path.display().to_string())
        .collect();

    if output_json {
        let _ = output::emit_result(
            "auth sync",
            output::obj(vec![
                ("auth_file", output::s(auth_file.display().to_string())),
                ("synced", output::n(synced as i64)),
                ("skipped", output::n(skipped as i64)),
                ("failed", output::n(failed as i64)),
                (
                    "updated_files",
                    output::arr(updated_files.into_iter().map(output::s).collect()),
                ),
            ]),
        );
    }

    0
}
