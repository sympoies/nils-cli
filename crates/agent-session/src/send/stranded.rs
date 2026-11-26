//! Content-free evidence for unkeyed terminal input that may remain in a pane.
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    CliContext, CliError, SessionRecord, send_guard::key, session_dir, write_private_file,
};

const SCHEMA: &str = "agent-session.stranded-input.v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    schema_version: String,
    session_id: String,
    runtime_id: String,
    attempt_id: String,
    pub(crate) input_sha256: String,
    pub(crate) record_updated_at: String,
    outcome: Outcome,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Unknown,
    Pasted,
    Submitted,
    Cleared,
}

fn path(context: &CliContext, record: &SessionRecord) -> PathBuf {
    session_dir(context, &record.id).join("stranded-input.json")
}

fn store(context: &CliContext, record: &SessionRecord, evidence: &Record) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(evidence).map_err(|_| {
        CliError::runtime(
            "stranded-record-failed",
            "could not encode stranded input evidence",
            None,
        )
    })?;
    write_private_file(&path(context, record), &bytes)
}

/// Start under the session lock before paste; unknown survives an interrupted write.
pub(crate) fn begin(
    context: &CliContext,
    record: &SessionRecord,
    text: &str,
) -> Result<Record, CliError> {
    let runtime_id = record
        .runtime
        .as_ref()
        .map(|r| r.launch_id.as_str())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            CliError::runtime(
                "send-identity-unobservable",
                "runtime identity is unavailable",
                None,
            )
        })?;
    let evidence = Record {
        schema_version: SCHEMA.into(),
        session_id: record.id.clone(),
        runtime_id: runtime_id.into(),
        attempt_id: uuid::Uuid::new_v4().to_string(),
        input_sha256: key::input_digest(text),
        record_updated_at: record.updated_at.clone(),
        outcome: Outcome::Unknown,
    };
    store(context, record, &evidence)?;
    Ok(evidence)
}

pub(crate) fn load(context: &CliContext, record: &SessionRecord) -> Option<Record> {
    load_with_modified(context, record).map(|(evidence, _)| evidence)
}

pub(crate) fn load_with_modified(
    context: &CliContext,
    record: &SessionRecord,
) -> Option<(Record, std::time::SystemTime)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path(context, record))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let evidence: Record = serde_json::from_slice(&bytes).ok()?;
    (evidence.schema_version == SCHEMA
        && evidence.session_id == record.id
        && record
            .runtime
            .as_ref()
            .is_some_and(|r| !r.launch_id.is_empty() && r.launch_id == evidence.runtime_id))
    .then_some((evidence, metadata.modified().ok()?))
}

impl Record {
    pub(crate) fn active(&self) -> bool {
        matches!(self.outcome, Outcome::Unknown | Outcome::Pasted)
    }
    pub(crate) fn pasted(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
    ) -> Result<(), CliError> {
        self.outcome = Outcome::Pasted;
        store(context, record, self)
    }
    pub(crate) fn submitted(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
    ) -> Result<(), CliError> {
        self.outcome = Outcome::Submitted;
        store(context, record, self)
    }
    pub(crate) fn clear_if_current(
        &self,
        context: &CliContext,
        record: &SessionRecord,
    ) -> Result<(), CliError> {
        if load(context, record).as_ref() == Some(self) {
            let mut cleared = self.clone();
            cleared.outcome = Outcome::Cleared;
            store(context, record, &cleared)?;
        }
        Ok(())
    }
}

/// A late provider acknowledgement must not resolve a newer attempt.
pub(crate) fn confirm(context: &CliContext, id: &str, expected: &Record) -> Result<(), CliError> {
    let _lock = crate::acquire_session_record_lock(context, id)?;
    let current = crate::load_session_record(context, id)?;
    if load(context, &current).as_ref() == Some(expected) {
        let mut submitted = expected.clone();
        submitted.submitted(context, &current)?;
    }
    Ok(())
}
