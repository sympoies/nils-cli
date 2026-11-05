//! Logical checkpoints for a Codex notice candidate, including hidden blank cells.
use super::{known_multiline_input, styled_text};
use unicode_width::UnicodeWidthChar;

pub(crate) struct NoticeCursorPath {
    pub(crate) characters: usize,
    pub(crate) start: (usize, usize),
    pub(crate) end: (usize, usize),
    pub(crate) rows: Vec<(usize, (usize, usize))>,
}

/// Normal captures retain physical cursor coordinates. A row that cannot be
/// reconstructed exactly (including an unsupported wrap) cannot authorize keys.
pub(crate) fn codex_notice_cursor_path(pane: &str, text: &str) -> Option<NoticeCursorPath> {
    if known_multiline_input("codex", pane, text).as_deref() != Some(text) {
        return None;
    }
    let (plain, _) = styled_text(pane)?;
    let lines: Vec<_> = plain.lines().collect();
    let start = lines.iter().rposition(|row| row.starts_with('›'))?;
    let visual = text.replace("\r\n", "\n");
    let logical_rows: Vec<_> = visual.split('\n').collect();
    let mut rows = Vec::with_capacity(logical_rows.len());
    for (index, row) in logical_rows.iter().enumerate() {
        let width = row.chars().try_fold(0, |cells, ch| match ch.width() {
            Some(width @ (1 | 2)) => Some(cells + width),
            _ => None,
        })?;
        let last = index + 1 == logical_rows.len();
        rows.push(if last {
            (row.chars().count(), (2 + width, start + index))
        } else {
            (row.chars().count() + 1, (2, start + index + 1))
        });
    }
    Some(NoticeCursorPath {
        characters: visual.chars().count(),
        start: (2, start),
        end: rows.last()?.1,
        rows,
    })
}
