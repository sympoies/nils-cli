use super::*;

pub(in super::super) fn read_messages_incremental_from_reader<R: Read + Seek>(
    session: &HistorySession,
    reader: &mut R,
    file_len: u64,
    offset: u64,
    max_bytes: usize,
    discarding_oversized_line: bool,
) -> Result<IncrementalReaderPage, HistoryError> {
    if offset > file_len || max_bytes == 0 {
        return Err(HistoryError::InvalidCursor);
    }
    if offset == file_len {
        return Ok(IncrementalReaderPage {
            messages: Vec::new(),
            next_offset: offset,
            discarding_oversized_line: false,
        });
    }
    let requested = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .min(file_len.saturating_sub(offset));
    let requested = usize::try_from(requested).map_err(|_| HistoryError::Io)?;
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| HistoryError::Io)?;
    let mut bytes = vec![0; requested];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| HistoryError::Io)?;
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    if complete == 0 {
        if !discarding_oversized_line {
            match read_extended_line(reader, file_len, offset, &bytes)? {
                ImageLineRead::Complete(line, next_offset) => {
                    let mut messages = Vec::new();
                    let pending = push_incremental_message(session, &line, offset, &mut messages);
                    return Ok(IncrementalReaderPage {
                        messages,
                        next_offset: if pending { offset } else { next_offset },
                        discarding_oversized_line: false,
                    });
                }
                ImageLineRead::Incomplete => {
                    return Ok(IncrementalReaderPage {
                        messages: Vec::new(),
                        next_offset: offset,
                        discarding_oversized_line: false,
                    });
                }
                ImageLineRead::Unsupported => {}
            }
        }
        let oversized = discarding_oversized_line || bytes.len() > MAX_LINE_BYTES;
        return Ok(IncrementalReaderPage {
            messages: Vec::new(),
            next_offset: if oversized {
                offset.saturating_add(bytes.len() as u64)
            } else {
                offset
            },
            discarding_oversized_line: oversized,
        });
    }
    let mut messages = Vec::new();
    let mut lines = bytes[..complete].split_inclusive(|byte| *byte == b'\n');
    let mut line_offset = offset;
    if discarding_oversized_line && let Some(line) = lines.next() {
        line_offset += line.len() as u64;
    }
    for raw_line in lines {
        let current_offset = line_offset;
        line_offset += raw_line.len() as u64;
        let line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.len() > MAX_IMAGE_RECORD_BYTES {
            continue;
        }
        if push_incremental_message(session, line, current_offset, &mut messages) {
            return Ok(IncrementalReaderPage {
                messages,
                next_offset: current_offset,
                discarding_oversized_line: false,
            });
        }
    }
    Ok(IncrementalReaderPage {
        messages,
        next_offset: offset.saturating_add(complete as u64),
        discarding_oversized_line: false,
    })
}
