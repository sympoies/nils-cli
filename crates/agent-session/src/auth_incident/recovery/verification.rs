//! Read-only proof of provider acceptance for an exact broker publication.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use nils_common::provider_usage::ProviderUsageReason;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude_account::{self, generation};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Failure {
    MaterializationRefused,
    PublicationChanged,
    VerificationInterrupted,
    ProviderHelperUnavailable,
    ProviderResponseUnverified,
    Provider(ProviderUsageReason),
}

impl Failure {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::MaterializationRefused => "materialization_refused",
            Self::PublicationChanged => "publication_changed",
            Self::VerificationInterrupted => "verification_interrupted",
            Self::ProviderHelperUnavailable => "provider_helper_unavailable",
            Self::ProviderResponseUnverified => "provider_response_unverified",
            Self::Provider(reason) => reason.as_str(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Verification {
    Started,
    Verified,
    Failed(Failure),
}

pub(super) fn current(publication: &generation::Publication) -> bool {
    generation::published(&publication.account)
        .is_ok_and(|current| current.as_ref() == Some(publication))
}

pub(super) fn verify(
    session_dir: &Path,
    publication: &generation::Publication,
) -> Result<(), Failure> {
    let config_dir = claude_account::materialize(&publication.account)
        .map_err(|_| Failure::MaterializationRefused)?;
    if !current(publication) {
        return Err(Failure::PublicationChanged);
    }
    // The existing helper owns the OAuth transport and credential parsing.
    // A fresh private HOME/cache and no inherited auth, endpoint override,
    // proxy, or Keychain inputs rule out cached or different-account success.
    let scratch =
        tempfile::tempdir_in(session_dir).map_err(|_| Failure::ProviderHelperUnavailable)?;
    let mut command = Command::new("claude-cli");
    command
        .args(["diag", "rate-limits", "--format", "json"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", scratch.path())
        .env("CLAUDE_PROMPT_SEGMENT_CACHE_DIR", scratch.path())
        .env("CLAUDE_AUTH_KEYCHAIN", "off")
        .env("CLAUDE_CONFIG_DIR", &config_dir)
        .current_dir(scratch.path());
    let output =
        crate::run_output_with_timeout_and_strict_cap(command, Duration::from_secs(10), 64 * 1024)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::TimedOut => Failure::Provider(ProviderUsageReason::Timeout),
                _ => Failure::ProviderHelperUnavailable,
            })?;
    // Never project stderr, diagnostics, cache paths, or raw response fields.
    let value: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| Failure::ProviderResponseUnverified)?;
    if value["schema_version"] != "claude-cli.diag.rate-limits.v1"
        || value["command"] != "diag rate-limits"
        || value["mode"] != "single"
        || value["result"]["provider"] != "claude"
    {
        return Err(Failure::ProviderResponseUnverified);
    }
    if !output.status.success()
        || value["ok"] != true
        || value["result"]["ok"] != true
        || value["result"]["source"] != "network"
    {
        return Err(value["result"]["reason_code"]
            .as_str()
            .and_then(ProviderUsageReason::from_code)
            .map_or(Failure::ProviderResponseUnverified, Failure::Provider));
    }
    if !current(publication) {
        return Err(Failure::PublicationChanged);
    }
    Ok(())
}
