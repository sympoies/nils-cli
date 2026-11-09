//! Logical checkpoints for a Codex notice candidate, including hidden blank cells.
use super::{known_multiline_input, styled_text};
use unicode_width::UnicodeWidthChar;

pub(crate) struct NoticeCursorPath {
    pub(crate) characters: usize,
    pub(crate) start: (usize, usize),
    pub(crate) end: (usize, usize),
    pub(crate) rows: Vec<(usize, (usize, usize))>,
    pub(crate) wrapped_header_end: Option<(usize, usize)>,
}

/// Normal captures retain physical cursor coordinates. A row that cannot be
/// reconstructed exactly (including an unsupported wrap) cannot authorize keys.
pub(crate) fn codex_notice_cursor_path(pane: &str, text: &str) -> Option<NoticeCursorPath> {
    let (plain, _) = styled_text(pane)?;
    let lines: Vec<_> = plain.lines().collect();
    let start = lines.iter().rposition(|row| row.starts_with('›'))?;
    let visual = text.replace("\r\n", "\n");
    let logical_rows: Vec<_> = visual.split('\n').collect();
    let header = *logical_rows.first()?;
    let mut rest = header;
    let mut header_end = start;
    let header_cells = loop {
        let row = if header_end == start {
            lines.get(header_end)?.strip_prefix("› ")?
        } else {
            lines.get(header_end)?.strip_prefix("  ")?
        };
        rest = rest.strip_prefix(row)?;
        if rest.is_empty() {
            break cells(row)?;
        }
        if row.is_empty() {
            return None;
        }
        // Codex 0.162.1 hangs word-separator spaces off the preceding
        // visual row. End/Home below must still prove one logical header.
        rest = rest.trim_start_matches(' ');
        header_end += 1;
    };
    let wrapped_rows = header_end - start;
    let mut logical_pane = lines[..start].join("\n");
    if start > 0 {
        logical_pane.push('\n');
    }
    logical_pane.push_str(&format!("› {header}\n"));
    logical_pane.push_str(&lines[header_end + 1..].join("\n"));
    if known_multiline_input("codex", &logical_pane, text).as_deref() != Some(text) {
        return None;
    }
    let mut rows = Vec::with_capacity(logical_rows.len());
    for (index, row) in logical_rows.iter().enumerate() {
        let width = cells(row)?;
        let last = index + 1 == logical_rows.len();
        rows.push(if last {
            (
                row.chars().count(),
                (2 + width, start + wrapped_rows + index),
            )
        } else {
            (
                row.chars().count() + 1,
                (2, start + wrapped_rows + index + 1),
            )
        });
    }
    Some(NoticeCursorPath {
        characters: visual.chars().count(),
        start: (2, start),
        end: rows.last()?.1,
        rows,
        wrapped_header_end: (wrapped_rows > 0).then_some((2 + header_cells, header_end)),
    })
}

fn cells(text: &str) -> Option<usize> {
    text.chars().try_fold(0, |cells, ch| match ch.width() {
        Some(width @ (1 | 2)) => Some(cells + width),
        _ => None,
    })
}
