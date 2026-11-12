//! Boundary-anchored compactions share one bounded adjacent lookup in all paging directions.
use super::*;
use crate::provider_prompt::row_classes as classes;
mod compact_order;

pub(super) enum Projection {
    Ordinary,
    Hidden,
    Pending,
    Row(Box<HistoryMessage>),
}

pub(super) fn project(
    session: &HistorySession,
    value: &Value,
    offset: u64,
    parts: bool,
) -> Projection {
    if session.provider == "claude"
        && classes::claude_boundary(value)
        && classes::claude_foreign(value, &session.provider_session_id)
    {
        return Projection::Hidden;
    }
    if session.provider == "claude"
        && let Some(projection) = compact_order::project(session, value, offset, parts)
    {
        return projection;
    }
    let summary = match session.provider.as_str() {
        "codex" if value["type"] == "compacted" => value
            .pointer("/payload/message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        "claude" if classes::claude_boundary(value) => {
            let Some(summary) = adjacent_summary(session, offset) else {
                return Projection::Pending;
            };
            summary
        }
        _ => return Projection::Ordinary,
    };
    Projection::Row(Box::new(compaction(
        session, value, offset, parts, &summary,
    )))
}

fn compaction(
    session: &HistorySession,
    value: &Value,
    offset: u64,
    parts: bool,
    summary: &str,
) -> HistoryMessage {
    let (text, compaction) = classes::summary(summary);
    HistoryMessage {
        id: format!("{}-{offset}", session.id),
        paging_offset: offset,
        role: "notice".into(),
        text,
        row_class: Some(classes::RowClass::Compaction),
        compaction: Some(compaction),
        timestamp: projected_timestamp(value),
        parts: parts.then(Vec::new),
        ..Default::default()
    }
}

fn adjacent_summary(session: &HistorySession, offset: u64) -> Option<String> {
    let mut file = fs::File::open(&session.transcript_path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut reader = BufReader::new(file.take(classes::ADJACENT_BYTES as u64));
    let mut remaining = classes::ADJACENT_BYTES;
    let mut line = Vec::new();
    // Include the anchor bytes in the fixed window, and require complete JSONL records.
    reader.read_until(b'\n', &mut line).ok()?;
    remaining = remaining.saturating_sub(line.len());
    if line.last() != Some(&b'\n') {
        return None;
    }
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).ok()?;
        remaining = remaining.saturating_sub(read);
        if line.last() != Some(&b'\n') {
            // A bound that cannot resolve the association has a stable empty summary.
            return (remaining == 0).then(String::new);
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if classes::claude_foreign(&value, &session.provider_session_id)
            || value["isSidechain"] == true
        {
            continue;
        }
        if let Some(summary) = classes::claude_summary(&value) {
            return Some(summary);
        }
        if classes::claude_logical_record(&value) {
            return Some(String::new());
        }
    }
}

pub(super) fn text_with_receipts(
    provider: &str,
    role: &str,
    text: &str,
    value: &Value,
    legacy_header: bool,
) -> (String, Option<classes::RowClass>) {
    let (text, class) = self::text(provider, role, text, value);
    if !legacy_header
        && class == Some(classes::RowClass::Notice)
        && text.starts_with("<!-- agent-notice:")
        && crate::notice::parse_notice(&text).is_some()
    {
        (truncate_chars(&text, MAX_MESSAGE_CHARS), None)
    } else {
        (text, class)
    }
}

pub(super) fn text(
    provider: &str,
    role: &str,
    text: &str,
    value: &Value,
) -> (String, Option<classes::RowClass>) {
    let class = (role == "user" && classes::runtime_command(provider, text))
        .then_some(classes::RowClass::RuntimeCommand);
    let class = class.or_else(|| {
        (role == "user" && classes::notice::matches(provider, text, value))
            .then_some(classes::RowClass::Notice)
    });
    if class == Some(classes::RowClass::Notice) {
        return (text.to_owned(), class);
    }
    if class.is_some() {
        return (classes::bounded_text(text).0, class);
    }
    // Classify the complete logical input before the history text bound.
    (
        truncate_chars(
            if role == "user" { text } else { text.trim() },
            MAX_MESSAGE_CHARS,
        ),
        class,
    )
}
