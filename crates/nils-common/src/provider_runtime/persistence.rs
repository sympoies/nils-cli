use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::fs as shared_fs;

use super::auth;
use super::paths;
use super::profile::ProviderProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampPolicy {
    Strict,
    BestEffort,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncSecretsResult {
    pub auth_file_present: bool,
    pub auth_identity_present: bool,
    pub synced: usize,
    pub skipped: usize,
    pub updated_files: Vec<PathBuf>,
}

impl SyncSecretsResult {
    fn auth_file_missing() -> Self {
        Self {
            auth_file_present: false,
            auth_identity_present: false,
            synced: 0,
            skipped: 0,
            updated_files: Vec::new(),
        }
    }

    fn auth_identity_missing() -> Self {
        Self {
            auth_file_present: true,
            auth_identity_present: false,
            synced: 0,
            skipped: 0,
            updated_files: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum SyncSecretsError {
    #[error("failed to hash auth file {path}: {source}")]
    HashAuthFile {
        path: PathBuf,
        #[source]
        source: shared_fs::FileHashError,
    },
    #[error("failed to read auth file {path}: {source}")]
    ReadAuthFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to hash secret file {path}: {source}")]
    HashSecretFile {
        path: PathBuf,
        #[source]
        source: shared_fs::FileHashError,
    },
    #[error("failed to write secret file {path}: {source}")]
    WriteSecretFile {
        path: PathBuf,
        #[source]
        source: shared_fs::AtomicWriteError,
    },
    #[error("failed to write timestamp file {path}: {source}")]
    WriteTimestampFile {
        path: PathBuf,
        #[source]
        source: shared_fs::TimestampError,
    },
}

pub fn sync_auth_to_matching_secrets(
    profile: &ProviderProfile,
    auth_file: &Path,
    secret_file_mode: u32,
    timestamp_policy: TimestampPolicy,
) -> Result<SyncSecretsResult, SyncSecretsError> {
    if !auth_file.is_file() {
        return Ok(SyncSecretsResult::auth_file_missing());
    }

    let auth_key = match auth::identity_key_from_auth_file(auth_file).ok().flatten() {
        Some(value) => value,
        None => return Ok(SyncSecretsResult::auth_identity_missing()),
    };

    let auth_last_refresh = auth::last_refresh_from_auth_file(auth_file).ok().flatten();
    let auth_hash =
        shared_fs::sha256_file(auth_file).map_err(|source| SyncSecretsError::HashAuthFile {
            path: auth_file.to_path_buf(),
            source,
        })?;
    let auth_contents = fs::read(auth_file).map_err(|source| SyncSecretsError::ReadAuthFile {
        path: auth_file.to_path_buf(),
        source,
    })?;

    let mut result = SyncSecretsResult {
        auth_file_present: true,
        auth_identity_present: true,
        synced: 0,
        skipped: 0,
        updated_files: Vec::new(),
    };

    if let Some(secret_dir) = paths::resolve_secret_dir(profile)
        && let Ok(entries) = fs::read_dir(secret_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let candidate_key = match auth::identity_key_from_auth_file(&path).ok().flatten() {
                Some(value) => value,
                None => {
                    result.skipped += 1;
                    continue;
                }
            };
            if candidate_key != auth_key {
                result.skipped += 1;
                continue;
            }

            let secret_hash = shared_fs::sha256_file(&path).map_err(|source| {
                SyncSecretsError::HashSecretFile {
                    path: path.clone(),
                    source,
                }
            })?;
            if secret_hash == auth_hash {
                result.skipped += 1;
                continue;
            }

            shared_fs::write_atomic(&path, &auth_contents, secret_file_mode).map_err(|source| {
                SyncSecretsError::WriteSecretFile {
                    path: path.clone(),
                    source,
                }
            })?;
            write_timestamp_for_target(
                profile,
                &path,
                auth_last_refresh.as_deref(),
                timestamp_policy,
            )?;
            result.synced += 1;
            result.updated_files.push(path);
        }
    }

    write_timestamp_for_target(
        profile,
        auth_file,
        auth_last_refresh.as_deref(),
        timestamp_policy,
    )?;

    Ok(result)
}

fn write_timestamp_for_target(
    profile: &ProviderProfile,
    target_file: &Path,
    iso: Option<&str>,
    timestamp_policy: TimestampPolicy,
) -> Result<(), SyncSecretsError> {
    let Some(timestamp_path) = paths::resolve_secret_timestamp_path(profile, target_file) else {
        return Ok(());
    };
    match shared_fs::write_timestamp(&timestamp_path, iso) {
        Ok(()) => Ok(()),
        Err(source) => match timestamp_policy {
            TimestampPolicy::Strict => Err(SyncSecretsError::WriteTimestampFile {
                path: timestamp_path,
                source,
            }),
            TimestampPolicy::BestEffort => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{TimestampPolicy, sync_auth_to_matching_secrets};
    use crate::provider_runtime::{
        ExecInvocation, ExecProfile, HomePathSelection, PathsProfile, ProviderDefaults,
        ProviderEnvKeys, ProviderProfile,
    };
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use std::sync::atomic::AtomicBool;

    const HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
    const PAYLOAD_ALPHA: &str = "eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19";
    const PAYLOAD_BETA: &str = "eyJzdWIiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSIsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3VzZXJfaWQiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSJ9fQ";
    const SECRET_HOME: &[&str] = &[".config", "test-secrets"];
    const AUTH_HOME: &[&str] = &[".config", "test-auth.json"];
    static WARNED_INVALID_ALLOW_DANGEROUS: AtomicBool = AtomicBool::new(false);

    static TEST_PROFILE: ProviderProfile = ProviderProfile {
        provider_name: "test",
        env: ProviderEnvKeys {
            model: "TEST_MODEL",
            reasoning: "TEST_REASONING",
            allow_dangerous_enabled: "TEST_ALLOW_DANGEROUS",
            secret_dir: "TEST_SECRET_DIR",
            auth_file: "TEST_AUTH_FILE",
            secret_cache_dir: "TEST_SECRET_CACHE_DIR",
            starship_enabled: "TEST_STARSHIP_ENABLED",
            auto_refresh_enabled: "TEST_AUTO_REFRESH_ENABLED",
            auto_refresh_min_days: "TEST_AUTO_REFRESH_MIN_DAYS",
        },
        defaults: ProviderDefaults {
            model: "test-model",
            reasoning: "medium",
            starship_enabled: "false",
            auto_refresh_enabled: "false",
            auto_refresh_min_days: "5",
        },
        paths: PathsProfile {
            feature_name: "test",
            feature_tool_script: "test-tools.zsh",
            secret_dir_home: HomePathSelection::ModernOnly(SECRET_HOME),
            auth_file_home: HomePathSelection::ModernOnly(AUTH_HOME),
            secret_cache_home: None,
        },
        exec: ExecProfile {
            default_caller_prefix: "test",
            missing_prompt_label: "_test_exec_dangerous",
            binary_name: "test-bin",
            failed_exec_message_prefix: "test-tools: failed to run test exec",
            invocation: ExecInvocation::CodexStyle,
            warned_invalid_allow_dangerous: &WARNED_INVALID_ALLOW_DANGEROUS,
        },
    };

    fn token(payload: &str) -> String {
        format!("{HEADER}.{payload}.sig")
    }

    fn auth_json(
        payload: &str,
        account_id: &str,
        refresh_token: &str,
        last_refresh: &str,
    ) -> String {
        format!(
            r#"{{"tokens":{{"access_token":"{}","id_token":"{}","refresh_token":"{}","account_id":"{}"}},"last_refresh":"{}"}}"#,
            token(payload),
            token(payload),
            refresh_token,
            account_id,
            last_refresh
        )
    }

    #[test]
    fn sync_auth_to_matching_secrets_updates_only_identity_matches() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        let cache_dir = dir.path().join("cache");
        std::fs::create_dir_all(&secret_dir).expect("secrets");
        std::fs::create_dir_all(&cache_dir).expect("cache");

        let auth_file = dir.path().join("auth.json");
        let alpha = secret_dir.join("alpha.json");
        let beta = secret_dir.join("beta.json");
        std::fs::write(
            &auth_file,
            auth_json(
                PAYLOAD_ALPHA,
                "acct_001",
                "refresh_new",
                "2025-01-20T12:34:56Z",
            ),
        )
        .expect("auth");
        std::fs::write(
            &alpha,
            auth_json(
                PAYLOAD_ALPHA,
                "acct_001",
                "refresh_old",
                "2025-01-19T12:34:56Z",
            ),
        )
        .expect("alpha");
        std::fs::write(
            &beta,
            auth_json(
                PAYLOAD_BETA,
                "acct_002",
                "refresh_beta",
                "2025-01-18T12:34:56Z",
            ),
        )
        .expect("beta");
        std::fs::write(secret_dir.join("invalid.json"), "{invalid").expect("invalid");

        let _secret = EnvGuard::set(
            &lock,
            "TEST_SECRET_DIR",
            secret_dir.to_string_lossy().as_ref(),
        );
        let _cache = EnvGuard::set(
            &lock,
            "TEST_SECRET_CACHE_DIR",
            cache_dir.to_string_lossy().as_ref(),
        );

        let result = sync_auth_to_matching_secrets(
            &TEST_PROFILE,
            &auth_file,
            crate::fs::SECRET_FILE_MODE,
            TimestampPolicy::Strict,
        )
        .expect("sync");

        assert!(result.auth_file_present);
        assert!(result.auth_identity_present);
        assert_eq!(result.synced, 1);
        assert_eq!(result.skipped, 2);
        assert_eq!(result.updated_files, vec![alpha.clone()]);
        assert_eq!(
            std::fs::read(&alpha).expect("alpha"),
            std::fs::read(&auth_file).expect("auth")
        );
        assert_ne!(
            std::fs::read(&beta).expect("beta"),
            std::fs::read(&auth_file).expect("auth")
        );
        assert_eq!(
            std::fs::read_to_string(cache_dir.join("alpha.json.timestamp"))
                .expect("alpha timestamp"),
            "2025-01-20T12:34:56Z"
        );
        assert_eq!(
            std::fs::read_to_string(cache_dir.join("auth.json.timestamp")).expect("auth timestamp"),
            "2025-01-20T12:34:56Z"
        );
    }

    #[test]
    fn sync_auth_to_matching_secrets_reports_missing_identity() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        std::fs::create_dir_all(&secret_dir).expect("secrets");

        let auth_file = dir.path().join("auth.json");
        std::fs::write(&auth_file, r#"{"tokens":{"refresh_token":"only"}}"#).expect("auth");

        let _secret = EnvGuard::set(
            &lock,
            "TEST_SECRET_DIR",
            secret_dir.to_string_lossy().as_ref(),
        );
        let result = sync_auth_to_matching_secrets(
            &TEST_PROFILE,
            &auth_file,
            crate::fs::SECRET_FILE_MODE,
            TimestampPolicy::BestEffort,
        )
        .expect("sync");
        assert!(result.auth_file_present);
        assert!(!result.auth_identity_present);
        assert_eq!(result.synced, 0);
        assert_eq!(result.skipped, 0);
    }
}
