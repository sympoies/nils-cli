use super::*;

/// The changed-ID exception must not admit a turn already seen in the retained
/// metadata history. Ordinary completions keep their existing admission path.
pub(super) fn allows_reconciliation(
    path: &Path,
    document: &ActivityDocument,
    event: &TurnEvent,
) -> bool {
    let Some(current) = document.state.current_turn.as_ref() else {
        return true;
    };
    if !completion_admission::reconciles_claude_idle(document, event, current) {
        return true;
    }
    let Some(entries) = retained_entries(path) else {
        return false;
    };
    !entries.iter().any(|entry| {
        entry.applied != Some(false)
            && entry.event.runtime_id == event.runtime_id
            && entry.event.provider == event.provider
            && entry.event.provider_session_id == event.provider_session_id
            && entry.event.provider_turn_id == event.provider_turn_id
    })
}

/// Exact receipt dedupe is independent of application. Only the latest accepted
/// terminal evidence can retry recovery; an explicitly rejected receipt cannot.
pub(super) fn retry_admitted(
    path: &Path,
    document: &ActivityDocument,
    event: &TurnEvent,
    exact: bool,
) -> bool {
    if !matches!(
        event.kind,
        TurnEventKind::TurnCompleted | TurnEventKind::TurnFailed
    ) || document.state.current_turn.is_some()
        || document.state.phase != TurnPhase::Waiting
        || document.last_provider_event_kind.as_ref() != Some(&event.kind)
        || document.last_provider_event_provider.as_ref() != Some(&event.provider)
        || document.last_provider_event_turn_id != event.provider_turn_id
        || document.provider_session_id != event.provider_session_id
        || document.last_semantic_event.as_deref() != Some(semantic_event_key(event).as_str())
        || !document.state.last_turn.as_ref().is_some_and(|turn| {
            turn.outcome
                == if event.kind == TurnEventKind::TurnFailed {
                    "failed"
                } else {
                    "completed"
                }
                && (event.provider_turn_id.is_none()
                    || turn.provider_turn_id == event.provider_turn_id)
        })
    {
        return false;
    }
    let Some(entries) = retained_entries(path) else {
        return false;
    };
    entries.iter().any(|entry| {
        entry.applied != Some(false)
            && (!exact || entry.event.event_id == event.event_id)
            && entry.event.runtime_id == event.runtime_id
            && entry.event.provider == event.provider
            && entry.event.provider_session_id == event.provider_session_id
            && entry.event.provider_turn_id == event.provider_turn_id
            && entry.event.kind == event.kind
            && entry.event.source_kind == event.source_kind
            && entry.event.confidence == event.confidence
            && entry.event.failure_reason == event.failure_reason
            && document.last_provider_event_at.as_deref() == Some(entry.received_at.as_str())
            && document.last_semantic_event_at.as_deref() == Some(entry.received_at.as_str())
            && document.last_event_at.as_deref() == Some(entry.received_at.as_str())
            && document.state.source.kind == event.source_kind
            && document.state.source.provider.as_deref() == Some(event.provider.as_str())
            && document.state.source.confidence == event.confidence
    })
}

pub(super) fn retained_entries(path: &Path) -> Option<Vec<JournalEntry>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_JOURNAL_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_JOURNAL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || bytes.len() > MAX_JOURNAL_BYTES || bytes.last() != Some(&b'\n') {
        return None;
    }
    let contents = std::str::from_utf8(&bytes).ok()?;
    let mut entries = Vec::new();
    for line in contents.lines() {
        if line.is_empty() || entries.len() >= MAX_JOURNAL_EVENTS {
            return None;
        }
        entries.push(serde_json::from_str(line).ok()?);
    }
    Some(entries)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct JournalEntry {
    pub(super) received_at: String,
    pub(super) event: TurnEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) applied: Option<bool>,
}

#[cfg(test)]
pub(super) fn append_journal(
    path: &Path,
    event: &TurnEvent,
    received_at: &str,
) -> Result<(), CliError> {
    append_journal_entry(
        path,
        JournalEntry {
            received_at: received_at.to_string(),
            event: event.clone(),
            applied: Some(true),
        },
    )
}

pub(super) fn append_journal_entry(path: &Path, entry: JournalEntry) -> Result<(), CliError> {
    let mut entries = if path.is_file() {
        fs::read_to_string(path)
            .map_err(|err| activity_io_error("activity-journal-read-failed", path, err))?
            .lines()
            .filter_map(|line| serde_json::from_str::<JournalEntry>(line).ok())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if entries.iter().any(|existing| {
        existing.event.runtime_id == entry.event.runtime_id
            && existing.event.event_id == entry.event.event_id
    }) {
        return Ok(());
    }
    entries.push(entry);
    if entries.len() > MAX_JOURNAL_EVENTS {
        let drop_count = entries.len() - MAX_JOURNAL_EVENTS;
        entries.drain(0..drop_count);
    }
    loop {
        let mut bytes = Vec::new();
        for entry in &entries {
            serde_json::to_writer(&mut bytes, entry).map_err(|err| {
                CliError::runtime(
                    "activity-journal-render-failed",
                    format!("failed to render activity journal: {err}"),
                    None,
                )
            })?;
            bytes.push(b'\n');
        }
        if bytes.len() <= MAX_JOURNAL_BYTES || entries.len() <= 1 {
            return write_atomic(path, &bytes, SECRET_FILE_MODE).map_err(|err| {
                CliError::runtime(
                    "activity-journal-write-failed",
                    format!("activity journal write failed at {}: {err}", path.display()),
                    Some(json!({ "path": display_path(path) })),
                )
            });
        }
        entries.remove(0);
    }
}
