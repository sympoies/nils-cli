//! Projection of surviving-process close failures from read-only journal evidence.

use serde::Serialize;
use serde_json::Value;

use super::{RECORD_BYTES, SEGMENT_BYTES, check_private_dir};
use crate::{CliContext, CliError, SessionRecord, coordination};

const TERMINATION_FAILED: &str = "session-termination-failed";
const PROCESS_STILL_RUNNING: &str = "process-still-running";

#[derive(Serialize)]
pub(crate) struct FailedClose {
    pub(crate) session_id: String,
    pub(crate) reason: &'static str,
}

fn is_close(operation: &str) -> bool {
    matches!(operation, "delete" | "archive" | "stop")
}

pub(super) fn reason(operation: &str, error: &CliError) -> Option<&'static str> {
    (is_close(operation)
        && error.code() == TERMINATION_FAILED
        && error.details()?.get("reason")?.as_str()? == PROCESS_STILL_RUNNING)
        .then_some(PROCESS_STILL_RUNNING)
}

pub(super) fn journal_target<'a>(
    operation: &str,
    error: Option<&CliError>,
    before: Option<&'a SessionRecord>,
    after: Option<&'a SessionRecord>,
) -> Option<&'a SessionRecord> {
    if error.and_then(|error| reason(operation, error)).is_none() {
        return after.or(before);
    }
    // A close can lose its lock before the journal is written. A failure
    // sampled across a replacement cannot be attributed to either identity.
    let before = before?;
    let after = after?;
    (before.id == after.id
        && crate::same_runtime_identity(before.runtime.as_ref(), after.runtime.as_ref()))
    .then_some(after)
}

pub(crate) fn read(
    context: &CliContext,
    id: &str,
    incarnation: &str,
    generation: u64,
) -> Option<FailedClose> {
    crate::validate_id(id).ok()?;
    if incarnation.is_empty() {
        return None;
    }
    let root = context.state_dir.join("lifecycle");
    check_private_dir(&root).ok()?;
    let journal = root.join(id);
    check_private_dir(&journal).ok()?;
    for segment in ["current.jsonl", "previous.jsonl"] {
        let bytes = match coordination::read_private_file(&journal.join(segment), SEGMENT_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        if bytes.is_empty() {
            continue;
        }
        // A partial or invalid newest record may conceal a later close.
        let complete = bytes.strip_suffix(b"\n")?;
        for line in complete.rsplit(|byte| *byte == b'\n') {
            if line.len() > RECORD_BYTES {
                return None;
            }
            let row: Value = serde_json::from_slice(line).ok()?;
            if row["schema_version"] != "agent-session.lifecycle.v1"
                || row["session_id"] != id
                || row["session_incarnation"] != incarnation
                || row["session_generation"].as_u64() != Some(generation)
                || !row["operation"].as_str().is_some_and(is_close)
            {
                continue;
            }
            let result = &row["result"];
            if result["ok"] != false
                || result["code"] != TERMINATION_FAILED
                || result["reason"] != PROCESS_STILL_RUNNING
            {
                return None;
            }
            let runtime = crate::load_session_record(context, id).ok()?.runtime?;
            return (runtime.launch_id == incarnation && runtime.generation == generation).then(
                || FailedClose {
                    session_id: id.to_owned(),
                    reason: PROCESS_STILL_RUNNING,
                },
            );
        }
    }
    None
}
