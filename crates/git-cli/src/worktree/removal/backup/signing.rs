//! Authorize the logical snapshot commit without changing repository config.
use super::{CliError, caused_failure, failure, git_error_reason, lease, probe_error_reason};
use nils_common::forge_identity;
use serde_json::json;
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

const IDENTITY_RECOVERY: &str = "Target and index retained; run forge-cli identity doctor --operation commit, repair the selected identity or signing setup, and retry";

fn identity_failure(error: forge_identity::Error) -> CliError {
    // Identity codes are fixed by the owner; arbitrary child diagnostics and
    // credential-bearing error details must not reach the removal receipt.
    failure("snapshot commit identity authorization failed")
        .with_hint(IDENTITY_RECOVERY)
        .with_details(json!({"reason": error.code}))
}

pub(super) fn commit(target: &Path, args: &[&str]) -> Result<String, CliError> {
    let mut command = Command::new("git");
    command.args(args).current_dir(target);
    lease::sanitize_git_environment(&mut command);
    // As in semantic-commit's commit-tree installation, authorize the logical
    // commit once. Apply policy after sanitization so its command-local identity
    // and signing configuration survive into this bounded child only.
    let authorization = forge_identity::prepare_git_with_deadline(
        &mut command,
        Some(target),
        &["commit"],
        Some(Instant::now() + Duration::from_secs(15)),
    )
    .map_err(identity_failure)?;
    if authorization.is_some() {
        // commit-tree does not honor commit.gpgsign itself. An authorized commit
        // must sign even when the ambient repository does not request signing.
        command.arg("-S");
    }
    let result = (|| {
        let mut output = lease::removal_probe(&mut command).map_err(|error| {
            failure("bounded snapshot commit failed")
                .with_details(json!({"reason": probe_error_reason(&error)}))
        })?;
        if let Some(auth) = &authorization {
            auth.redact_output(&mut output);
        }
        if !output.status.success() {
            return Err(caused_failure(
                "snapshot commit could not be written",
                failure("snapshot Git operation failed")
                    .with_details(json!({"reason": git_error_reason(&output)})),
            ));
        }
        String::from_utf8(output.stdout)
            .map(|text| text.trim().to_owned())
            .map_err(|_| failure("snapshot commit output unreadable"))
    })();
    if let Some(auth) = &authorization {
        // Complete authorization for every execution outcome, including bounded
        // spawn/timeout failures, before the caller can publish the backup ref.
        auth.finish(result.is_ok(), result.as_deref().ok())
            .map_err(identity_failure)?;
    }
    result.map_err(|error| {
        if authorization.is_some() {
            error.with_hint(IDENTITY_RECOVERY)
        } else {
            error
        }
    })
}
