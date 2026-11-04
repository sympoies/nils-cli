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
            id: format!("{}-{}", session.id, pair.echo_offset),
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
        return None;
    }
    let mut row = compaction(session, &boundary, anchor, parts, &pair.summary);
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
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(offset - start).read_to_end(&mut bytes).ok()?;
    let mut end = offset;
    for line in bytes.split_inclusive(|byte| *byte == b'\n').rev() {
        end -= line.len() as u64;
        if end == start && start > 0 {
            break; // The first bounded fragment need not start at a record boundary.
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if ignored(session, &value) {
            continue;
        }
        if classes::claude_boundary(&value) {
            return Some((end, value));
        }
        if classes::claude_summary(&value).is_none() && classes::claude_logical_record(&value) {
            break;
        }
    }
    None
}
