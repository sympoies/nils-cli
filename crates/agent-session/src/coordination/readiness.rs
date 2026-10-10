use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nils_common::fs::{SECRET_FILE_MODE, display_path};
use serde_json::{Value, json};

use crate::{CliContext, CliError, SessionRecord, cli::ReadinessArgs};

pub(crate) fn run(context: &CliContext, args: ReadinessArgs) -> i32 {
    super::render_coordination("readiness", args.format, evaluate(context))
}

fn evaluate(context: &CliContext) -> Result<Value, CliError> {
    let (record, incarnation) =
        super::authenticate_any_from_file(context, None).map_err(|error| {
            if error.code() == "coordination-unauthorized" {
                CliError::data(error.code(), error.message(), Some(recovery_details()))
            } else {
                error
            }
        })?;
    let checkpoint_file =
        ensure_runtime_checkpoint_ready(context, &record, &incarnation).map_err(|mut error| {
            *error.details_mut() = Some(recovery_details());
            error
        })?;
    Ok(json!({
        "schema_version": "agent-session.runtime-readiness.v1",
        "ready": true,
        "session_id": record.id,
        "session_incarnation": incarnation,
        "checkpoint_file": display_path(&checkpoint_file)
    }))
}

pub fn checkpoint_path_for_state(state_dir: &Path, session_id: &str, incarnation: &str) -> PathBuf {
    state_dir
        .join("sessions")
        .join(session_id)
        .join("coordination")
        .join(format!(
            "main-agent-checkpoint-{}.json",
            super::digest_bytes(incarnation.as_bytes())
        ))
}

/// Verify the private checkpoint prepared by the broker for this incarnation.
/// This does not create, repair, read, or write checkpoint contents.
pub fn ensure_runtime_checkpoint_ready(
    context: &CliContext,
    record: &SessionRecord,
    incarnation: &str,
) -> Result<PathBuf, CliError> {
    let expected = checkpoint_path_for_state(&context.state_dir, &record.id, incarnation);
    let supplied = std::env::var_os(super::CHECKPOINT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(checkpoint_unavailable)?;
    if supplied != expected {
        return Err(checkpoint_unavailable());
    }
    let metadata = fs::symlink_metadata(&expected).map_err(|_| checkpoint_unavailable())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o777 != SECRET_FILE_MODE
    {
        return Err(checkpoint_unavailable());
    }
    Ok(expected)
}

fn checkpoint_unavailable() -> CliError {
    CliError::data(
        "runtime-checkpoint-unavailable",
        "this session incarnation has no trusted runtime-issued checkpoint file; resume or restart the managed session after deploying compatible runtime surfaces",
        Some(json!({"required_action": "resume-or-restart-managed-session"})),
    )
}

fn recovery_details() -> Value {
    json!({
        "required_action": "resume-or-restart-managed-session",
        "retryable": false,
        "next_action": "resume-or-restart-managed-session",
        "recovery": {"action": "resume-or-restart-managed-session"}
    })
}
