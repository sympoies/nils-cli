//! Keep provider row identity separate from the two physical paging positions.
use super::*;

struct Pair {
    echo: Value,
    echo_offset: u64,
    summary: String,
}

enum End {
    Pending,
    Unpaired,
}

pub(super) fn project(
    session: &HistorySession,
    value: &Value,
    offset: u64,
    parts: bool,
) -> Option<Projection> {
    if classes::claude_boundary(value) {
        if classes::claude_compaction_trigger(value) == classes::CompactionTrigger::Auto {
            return None;
        }
        let pair = match following_echo(session, offset) {
            Ok(pair) => pair,
            Err(End::Pending)
                if classes::claude_compaction_trigger(value)
                    == classes::CompactionTrigger::Manual =>
            {
                return Some(Projection::Pending);
            }
            Err(_) => return None,
        };
        let text = classes::claude_compact_echo(&pair.echo)?;
        return Some(Projection::Row(Box::new(HistoryMessage {
            id: format!("{}-{offset}", session.id),
            paging_offset: offset,
            role: "user".into(),
            text: classes::bounded_text(&text).0,
            row_class: Some(classes::RowClass::RuntimeCommand),
            timestamp: projected_timestamp(&pair.echo),
            parts: parts.then(Vec::new),
            ..Default::default()
        })));
    }
    classes::claude_compact_echo(value)?;
    if classes::claude_foreign(value, &session.provider_session_id) || value["isSidechain"] == true
    {
        return None;
    }
    let (anchor, boundary) = preceding_boundary(session, offset)?;
    if classes::claude_compaction_trigger(&boundary) == classes::CompactionTrigger::Auto {
        return None;
    }
    let pair = match following_echo(session, anchor) {
        Ok(pair) => pair,
        Err(End::Pending)
            if classes::claude_compaction_trigger(&boundary)
                == classes::CompactionTrigger::Manual =>
        {
            return Some(Projection::Pending);
        }
        Err(_) => return None,
    };
    if pair.echo_offset != offset {
        return Some(Projection::Hidden);
    }
    let mut row = compaction(session, &boundary, offset, parts, &pair.summary);
    row.paging_offset = offset;
    Some(Projection::Row(Box::new(row)))
}

fn ignored(session: &HistorySession, value: &Value) -> bool {
    classes::claude_foreign(value, &session.provider_session_id) || value["isSidechain"] == true
}

fn following_echo(session: &HistorySession, anchor: u64) -> Result<Pair, End> {
    let mut file = fs::File::open(&session.transcript_path).map_err(|_| End::Pending)?;
    file.seek(SeekFrom::Start(anchor))
        .map_err(|_| End::Pending)?;
    let mut reader = BufReader::new(file.take(classes::ADJACENT_BYTES as u64));
    let mut offset = anchor;
    let mut line = Vec::new();
    reader
        .read_until(b'\n', &mut line)
        .map_err(|_| End::Pending)?;
    if line.last() != Some(&b'\n') {
        return Err(End::Pending);
    }
    offset += line.len() as u64;
    let mut summary = None;
    loop {
        line.clear();
        reader
            .read_until(b'\n', &mut line)
            .map_err(|_| End::Pending)?;
        if line.last() != Some(&b'\n') {
            return Err(
                if offset - anchor + line.len() as u64 >= classes::ADJACENT_BYTES as u64 {
                    End::Unpaired
                } else {
                    End::Pending
                },
            );
        }
        let line_offset = offset;
        offset += line.len() as u64;
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if ignored(session, &value) {
            continue;
        }
        if let Some(text) = classes::claude_summary(&value) {
            if summary.replace(text).is_some() {
                return Err(End::Unpaired);
            }
        } else if classes::claude_compact_echo(&value).is_some() {
            return Ok(Pair {
                echo: value,
                echo_offset: line_offset,
                summary: summary.unwrap_or_default(),
            });
        } else if classes::claude_logical_record(&value) {
            return Err(End::Unpaired);
        }
    }
}

fn preceding_boundary(session: &HistorySession, offset: u64) -> Option<(u64, Value)> {
    let start = offset.saturating_sub(classes::ADJACENT_BYTES as u64);
    let mut file = fs::File::open(&session.transcript_path).ok()?;
    let mut cursor = offset;
    let mut pending = Vec::new();
    let mut summary_seen = false;
    let mut chunk_size = 8192_u64;
    while cursor > start {
        let chunk_start = cursor.saturating_sub(chunk_size).max(start);
        file.seek(SeekFrom::Start(chunk_start)).ok()?;
        let mut bytes = vec![0; (cursor - chunk_start) as usize];
        file.read_exact(&mut bytes).ok()?;
        bytes.extend_from_slice(&pending);
        let mut end = bytes.len();
        while end > 0 {
            let Some(delimiter) = bytes[..end.saturating_sub(1)]
                .iter()
                .rposition(|byte| *byte == b'\n')
            else {
                break;
            };
            let line_start = delimiter + 1;
            let line = &bytes[line_start..end];
            let line_offset = chunk_start + line_start as u64;
            end = line_start;
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            if ignored(session, &value) {
                continue;
            }
            if classes::claude_boundary(&value) {
                return Some((line_offset, value));
            }
            if classes::claude_summary(&value).is_some() {
                summary_seen = true;
                continue;
            }
            if !summary_seen && classes::claude_compact_echo(&value).is_some() {
                continue;
            }
            if classes::claude_logical_record(&value) {
                return None;
            }
        }
        pending = bytes[..end].to_vec();
        cursor = chunk_start;
        chunk_size = (chunk_size * 2).min(classes::ADJACENT_BYTES as u64);
    }
    // As in the bounded scan, only the start of the file is a known record boundary.
    if start == 0 {
        let value = serde_json::from_slice::<Value>(&pending).ok()?;
        if !ignored(session, &value) && classes::claude_boundary(&value) {
            return Some((0, value));
        }
    }
    None
}
