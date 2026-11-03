//! Explicit, metadata-only snapshots. Ordinary session projections never call this collector.
use std::{
    fs,
    fs::File,
    io::Seek,
    path::Path,
    time::{Duration, Instant, SystemTime},
};

use jiff::Timestamp;
use serde::Serialize;

use crate::{
    CliContext, SessionRecord,
    provider_prompt::{ProviderPromptSource, ProviderPromptTail},
};

mod counts;

const COUNT_DEADLINE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    NotBound,
    SourceUnavailable,
    IdentityChanged,
    ReadBudgetExceeded,
    IncompleteSource,
    UnsupportedFormat,
    AmbiguousCompletionEvidence,
    ProviderCreationUnavailable,
    InvalidTimestamp,
    NotRecorded,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Metric {
    value: Option<u64>,
    unknown_reason: Option<Reason>,
}

impl Metric {
    fn known(value: u64) -> Self {
        Self {
            value: Some(value),
            unknown_reason: None,
        }
    }
    fn unknown(reason: Reason) -> Self {
        Self {
            value: None,
            unknown_reason: Some(reason),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Provider {
    Claude,
    Codex,
    Dsh,
    Unknown,
}

#[derive(Serialize)]
struct Resume {
    kind: &'static str,
    outcome: &'static str,
    started_at: Option<String>,
    completed_at: Option<String>,
    deadline_at: Option<String>,
    duration_ms: Option<u64>,
    unknown_reason: Reason,
}

#[derive(Serialize)]
pub(crate) struct SessionMetrics {
    schema_version: &'static str,
    observed_at: String,
    provider: Provider,
    transcript_scope: &'static str,
    compaction_scope: &'static str,
    transcript_bytes: Metric,
    transcript_lines: Metric,
    completed_compactions: Metric,
    managed_age_seconds: Metric,
    conversation_age_seconds: Metric,
    managed_created_at: Option<String>,
    conversation_created_at: Option<String>,
    resume: Resume,
}

fn age(
    created: Option<Timestamp>,
    observed: Timestamp,
    missing: Reason,
) -> (Option<String>, Metric) {
    let Some(created) = created else {
        return (None, Metric::unknown(missing));
    };
    let observed: SystemTime = observed.into();
    let time: SystemTime = created.into();
    match observed.duration_since(time) {
        Ok(duration) => (Some(created.to_string()), Metric::known(duration.as_secs())),
        Err(_) => (None, Metric::unknown(Reason::InvalidTimestamp)),
    }
}

impl SessionMetrics {
    fn unavailable_file(&mut self, reason: Reason) {
        self.transcript_bytes = Metric::unknown(reason);
        self.transcript_lines = Metric::unknown(reason);
        self.completed_compactions = Metric::unknown(reason);
        if !matches!(self.provider, Provider::Claude) || reason == Reason::IdentityChanged {
            self.conversation_created_at = None;
            self.conversation_age_seconds = Metric::unknown(reason);
        }
    }
}

pub(crate) fn collect(context: &CliContext, record: &SessionRecord) -> SessionMetrics {
    collect_at(context, record, Timestamp::now(), || {})
}

fn collect_at(
    context: &CliContext,
    record: &SessionRecord,
    observed: Timestamp,
    after_scan: impl FnOnce(),
) -> SessionMetrics {
    let binding = binding(record);
    let (managed_created_at, managed_age_seconds) = age(
        record.created_at.parse().ok(),
        observed,
        Reason::InvalidTimestamp,
    );
    let mut result = SessionMetrics {
        schema_version: "agent-session.session-metrics.v1",
        observed_at: observed.to_string(),
        provider: match record.agent.as_str() {
            "claude" => Provider::Claude,
            "codex" => Provider::Codex,
            "dsh" => Provider::Dsh,
            _ => Provider::Unknown,
        },
        transcript_scope: "selected_primary_file",
        compaction_scope: "retained_transcript",
        transcript_bytes: Metric::unknown(Reason::NotBound),
        transcript_lines: Metric::unknown(Reason::NotBound),
        completed_compactions: Metric::unknown(Reason::NotBound),
        managed_created_at,
        managed_age_seconds,
        conversation_created_at: None,
        conversation_age_seconds: Metric::unknown(Reason::ProviderCreationUnavailable),
        resume: Resume {
            kind: "unknown",
            outcome: "unknown",
            started_at: None,
            completed_at: None,
            deadline_at: None,
            duration_ms: None,
            unknown_reason: Reason::NotRecorded,
        },
    };
    let facts =
        if !matches!(record.agent.as_str(), "claude" | "codex") {
            Err(Reason::UnsupportedFormat)
        } else if record.provider_resume.as_ref().is_none_or(|resume| {
            resume.provider != record.agent || resume.session_id.trim().is_empty()
        }) {
            Err(Reason::NotBound)
        } else if !crate::conversation::binding_is_current(context, record) {
            Err(Reason::IdentityChanged)
        } else {
            ProviderPromptTail::resolve_source(record)
                .ok_or(Reason::SourceUnavailable)
                .and_then(|source| read_file(&source, record, after_scan))
        };
    match facts {
        Ok(facts) => {
            result.transcript_bytes = Metric::known(facts.length);
            result.transcript_lines = facts.counts.lines;
            result.completed_compactions = facts.counts.compactions;
            (
                result.conversation_created_at,
                result.conversation_age_seconds,
            ) = age(facts.created, observed, Reason::ProviderCreationUnavailable);
        }
        Err(reason) => result.unavailable_file(reason),
    }
    match crate::load_session_record(context, &record.id) {
        Ok(current)
            if binding == self::binding(&current)
                && crate::conversation::binding_is_current(context, &current) => {}
        _ => {
            result.unavailable_file(Reason::IdentityChanged);
            result.managed_created_at = None;
            result.managed_age_seconds = Metric::unknown(Reason::IdentityChanged);
        }
    }
    result
}

// Used only for equality, never included in responses, errors or evidence.
fn binding(record: &SessionRecord) -> serde_json::Value {
    serde_json::json!((
        &record.id,
        &record.created_at,
        &record.agent,
        record
            .runtime
            .as_ref()
            .map(|r| (&r.launch_id, r.generation)),
        record
            .provider_resume
            .as_ref()
            .map(|r| (&r.provider, &r.session_id)),
        crate::session_provider_config_dir(record),
        crate::session_effective_provider_config_dir(record),
        crate::session_agent_profile(record),
        crate::codex_sessions_root(),
        crate::claude_projects_root(),
    ))
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    length: u64,
    modified: SystemTime,
    device: u64,
    inode: u64,
    changed_seconds: i64,
    changed_nanos: i64,
}

fn fingerprint(metadata: &fs::Metadata) -> Result<Fingerprint, Reason> {
    use std::os::unix::fs::MetadataExt;
    if !metadata.file_type().is_file() {
        return Err(Reason::SourceUnavailable);
    }
    Ok(Fingerprint {
        length: metadata.len(),
        modified: metadata.modified().map_err(|_| Reason::SourceUnavailable)?,
        device: metadata.dev(),
        inode: metadata.ino(),
        changed_seconds: metadata.ctime(),
        changed_nanos: metadata.ctime_nsec(),
    })
}

struct FileFacts {
    length: u64,
    counts: counts::Counts,
    created: Option<Timestamp>,
}

fn read_file(
    source: &ProviderPromptSource,
    record: &SessionRecord,
    after_scan: impl FnOnce(),
) -> Result<FileFacts, Reason> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = source.path();
    let before = fingerprint(&fs::symlink_metadata(path).map_err(|_| Reason::SourceUnavailable)?)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Reason::SourceUnavailable)?;
    if fingerprint(&file.metadata().map_err(|_| Reason::SourceUnavailable)?)? != before {
        return Err(Reason::IdentityChanged);
    }
    read_verified_file(source, record, file, before, after_scan)
}

fn read_verified_file(
    source: &ProviderPromptSource,
    record: &SessionRecord,
    mut file: File,
    before: Fingerprint,
    after_scan: impl FnOnce(),
) -> Result<FileFacts, Reason> {
    let path = source.path();
    // Reuse the existing bounded identity/header owners once for this snapshot.
    let resume = record.provider_resume.as_ref().ok_or(Reason::NotBound)?;
    let created = if record.agent == "codex" {
        let meta = nils_provider_resume::read_codex_resumable_session_meta_from_file(&mut file)
            .filter(|meta| meta.session_id == resume.session_id)
            .ok_or(Reason::IdentityChanged)?;
        file.rewind().map_err(|_| Reason::SourceUnavailable)?;
        Some(Timestamp::try_from(meta.created_at).map_err(|_| Reason::InvalidTimestamp)?)
    } else {
        source.validated_path().ok_or(Reason::IdentityChanged)?;
        None
    };
    let deadline = Instant::now() + COUNT_DEADLINE;
    #[cfg(test)]
    record_scan(path);
    let counts = counts::scan(
        &mut file,
        before.length,
        &record.agent,
        &resume.session_id,
        || Instant::now() >= deadline,
    );
    after_scan();
    if !still_current(path, &file, &before) {
        return Err(Reason::IdentityChanged);
    }
    Ok(FileFacts {
        length: before.length,
        counts,
        created,
    })
}

fn still_current(path: &Path, file: &File, before: &Fingerprint) -> bool {
    fs::symlink_metadata(path)
        .ok()
        .and_then(|m| fingerprint(&m).ok())
        .as_ref()
        == Some(before)
        && file
            .metadata()
            .ok()
            .and_then(|m| fingerprint(&m).ok())
            .as_ref()
            == Some(before)
}

#[cfg(test)]
fn scan_counts() -> &'static std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, usize>>
{
    static COUNTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, usize>>,
    > = std::sync::OnceLock::new();
    COUNTS.get_or_init(Default::default)
}

#[cfg(test)]
fn record_scan(path: &Path) {
    *scan_counts()
        .lock()
        .unwrap()
        .entry(path.to_path_buf())
        .or_default() += 1;
}

#[cfg(test)]
pub(crate) fn scans_for(path: &Path) -> usize {
    scan_counts()
        .lock()
        .unwrap()
        .get(path)
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
