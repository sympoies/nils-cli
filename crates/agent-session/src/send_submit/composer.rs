//! Exact composer observation, retaining dim styling for native empty hints.
use super::{
    CLAUDE_QUEUED_HINT, DIALOG_HINTS, PASTE_PLACEHOLDERS, claude_composer, codex_composer, compact,
};
use unicode_width::UnicodeWidthChar;

mod command_ui;
pub(crate) use command_ui::observed as command_ui_observed;

pub(crate) fn exact_composer_input(agent: &str, pane: &str) -> Option<String> {
    composer_input(agent, pane, false)
}

/// Match a known multiline notice only in a logical capture where tmux has
/// joined soft-wrapped rows. Unknown drafts are never inferred from a prefix.
pub(crate) fn known_multiline_input(agent: &str, pane: &str, expected: &str) -> Option<String> {
    crate::notice::parse_notice(expected)?;
    let visual = expected.replace("\r\n", "\n");
    if visual.contains('\r') {
        return None;
    }
    let (plain, _) = styled_text(pane)?;
    let lines: Vec<&str> = plain.lines().collect();
    let (start, region, footer) = match agent {
        "claude" => {
            let (region, footer) = claude_composer(&lines)?;
            let start = lines
                .iter()
                .position(|line| std::ptr::eq(*line, region[0]))?;
            (start, region, footer)
        }
        "codex" => {
            let start = lines.iter().rposition(|line| line.starts_with('›'))?;
            let mut end = lines.len();
            while end > start + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            let below = &lines[start + 1..end];
            let region_end = below
                .iter()
                .rposition(|line| line.trim().is_empty())
                .map_or(end, |blank| start + 1 + blank);
            (start, &lines[start..region_end], &lines[region_end..])
        }
        _ => return None,
    };
    if footer.iter().any(|line| {
        let lower = line.to_lowercase();
        DIALOG_HINTS.iter().any(|hint| lower.contains(hint))
    }) {
        return None;
    }
    if agent == "codex"
        && footer.iter().filter(|line| !line.trim().is_empty()).count()
            > super::CODEX_MAX_FOOTER_LINES
    {
        return None;
    }
    let glyph = if agent == "claude" { '❯' } else { '›' };
    let first = lines[start].trim_start().strip_prefix(glyph)?;
    let first = first
        .strip_prefix([' ', '\u{a0}', '\u{2007}', '\u{202f}'])
        .unwrap_or(first);
    let mut observed_rows = Vec::with_capacity(region.len());
    observed_rows.push(first);
    observed_rows.extend(region.iter().skip(1).copied());
    let expected_rows: Vec<&str> = visual.split('\n').collect();
    if agent == "claude" && expected_rows.iter().any(|row| row.ends_with(' ')) {
        return None;
    }
    let mut logical_rows = Vec::with_capacity(observed_rows.len());
    for (index, observed) in observed_rows.iter().enumerate() {
        let wanted = expected_rows.get(index).copied();
        if index > 0 && observed.is_empty() {
            logical_rows.push(String::new());
            continue;
        }
        if index > 0 && observed.starts_with("  ") && wanted.is_some_and(str::is_empty) {
            // Spaces on a logically empty row cannot be distinguished from
            // prompt indentation, so do not infer an exact empty line.
            return None;
        }
        let without_indent = (index > 0).then(|| observed.strip_prefix("  ")).flatten();
        let raw_matches = wanted.is_some_and(|wanted| *observed == wanted);
        let indented_matches = without_indent.is_some_and(|row| wanted == Some(row));
        let row = match (raw_matches, indented_matches) {
            (true, false) => *observed,
            (false, true) => without_indent.unwrap_or(observed),
            (false, false) if index > 0 && without_indent.is_some() => {
                without_indent.unwrap_or(observed)
            }
            (false, false) => *observed,
            (true, true) => return None,
        };
        logical_rows.push(row.to_owned());
    }
    let actual = logical_rows.join("\n");
    if actual == visual {
        return Some(expected.to_owned());
    }
    if agent == "claude"
        && let Some(last) = logical_rows.last()
        && let Some(expected_last) = expected_rows.last()
        && logical_rows.len() == expected_rows.len()
        && logical_rows[..logical_rows.len() - 1]
            .iter()
            .zip(&expected_rows[..expected_rows.len() - 1])
            .all(|(actual, expected)| actual == expected)
        && last
            .strip_prefix(expected_last)
            .is_some_and(|suffix| suffix.chars().all(|ch| ch == ' '))
    {
        // Claude pads the active row after the insertion point. The caller
        // confirms that cursor position before admitting this candidate.
        return Some(expected.to_owned());
    }
    Some(actual)
}

/// Confirm the current Claude insertion point ends at the matched final row.
/// Pane padding after the cursor is rendering, while any additional owner text
/// before the cursor changes the visible suffix and fails this check.
pub(crate) fn known_multiline_cursor_holds(
    agent: &str,
    pane: &str,
    cursor: (usize, usize),
    expected: &str,
) -> bool {
    if agent != "claude" {
        return true;
    }
    let visual = expected.replace("\r\n", "\n");
    let Some(expected_last_row) = visual.rsplit('\n').next() else {
        return false;
    };
    let Some((plain, _)) = styled_text(pane) else {
        return false;
    };
    let lines: Vec<&str> = plain.lines().collect();
    let Some((region, _)) = claude_composer(&lines) else {
        return false;
    };
    let Some(last_row_index) = lines
        .iter()
        .position(|line| region.last().is_some_and(|last| std::ptr::eq(*line, *last)))
    else {
        return false;
    };
    if cursor.1 != last_row_index {
        return false;
    }
    let Some(row) = lines.get(cursor.1) else {
        return false;
    };
    let first_row_index = lines.iter().position(|line| std::ptr::eq(*line, region[0]));
    let mut interpretations = Vec::new();
    if first_row_index == Some(cursor.1) {
        let Some(after_glyph) = row.trim_start().strip_prefix('❯') else {
            return false;
        };
        let Some(content) = after_glyph.strip_prefix([' ', '\u{a0}', '\u{2007}', '\u{202f}'])
        else {
            return false;
        };
        interpretations.push((content, 2));
    } else {
        interpretations.push((*row, 2));
        if let Some(content) = row.strip_prefix("  ") {
            interpretations.push((content, 2));
        }
    }
    let mut matches = 0;
    for (content, start_column) in interpretations {
        let Some(cell_count) = cursor.0.checked_sub(start_column) else {
            continue;
        };
        if let Some(prefix) = prefix_at_cells(content, cell_count)
            && expected_last_row.ends_with(prefix)
        {
            matches += 1;
        }
    }
    matches == 1
}

fn prefix_at_cells(text: &str, cells: usize) -> Option<&str> {
    if cells == 0 {
        return Some("");
    }
    let mut width = 0;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        width += UnicodeWidthChar::width(ch)?;
        if width > cells {
            return None;
        }
        end = index + ch.len_utf8();
    }
    (width == cells).then_some(&text[..end])
}

/// Direct replies may follow a queued message when the sole input row is a
/// native dim-styled queued hint. Literal or adjacent owner input stays a draft.
pub(crate) fn direct_composer_input(agent: &str, pane: &str) -> Option<String> {
    composer_input(agent, pane, true)
}

fn composer_input(agent: &str, pane: &str, allow_queued_hint: bool) -> Option<String> {
    let (plain, dim) = styled_text(pane)?;
    let lines: Vec<&str> = plain.lines().collect();
    let (region, footer) = match agent {
        "claude" => claude_composer(&lines),
        "codex" => codex_composer(&lines),
        _ => None,
    }?;
    if region.len() != 1
        || footer.iter().any(|line| {
            let lower = line.to_lowercase();
            DIALOG_HINTS.iter().any(|hint| lower.contains(hint))
        })
    {
        return None;
    }
    let glyph = if agent == "claude" { '❯' } else { '›' };
    let after_glyph = region[0].trim_start().strip_prefix(glyph)?;
    let input = after_glyph
        .strip_prefix([' ', '\u{a0}', '\u{2007}', '\u{202f}'])
        .unwrap_or(after_glyph);
    let row = lines
        .iter()
        .position(|line| std::ptr::eq(*line, region[0]))?;
    let offset = lines[..row]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>()
        + region[0].len()
        - input.len();
    // Claude writes spaces after its input up to the pane width, in every
    // capture mode, for empty inputs, suggestions and typed text alike. Written
    // padding and typed trailing spaces cannot be told apart, so unstyled
    // trailing ASCII spaces are not input. Any other blank stays input.
    let input = if agent == "claude" {
        let padding = input
            .bytes()
            .rev()
            .enumerate()
            .take_while(|(back, byte)| *byte == b' ' && !dim[offset + input.len() - 1 - back])
            .count();
        &input[..input.len() - padding]
    } else {
        input
    };
    // Codex can leave unstyled layout cells after its native dim hint. Ignore
    // only trailing spaces without dim styling; styled whitespace and any
    // unstyled draft text still prevent an empty classification.
    let mut hint_end = input.len();
    if agent == "codex" {
        while input[..hint_end].ends_with(' ') && !dim[offset + hint_end - 1] {
            hint_end -= 1;
        }
    }
    let fully_dim = dim_hint(&input[..hint_end], &dim[offset..offset + hint_end]);
    let compressed = compact(input);
    if compressed == CLAUDE_QUEUED_HINT {
        if !allow_queued_hint || agent != "claude" {
            return None;
        }
        return Some(if fully_dim {
            String::new()
        } else {
            input.to_string()
        });
    }
    if PASTE_PLACEHOLDERS
        .iter()
        .any(|mark| compressed.contains(mark))
    {
        return None;
    }
    // Native suggestions are dynamic, not limited to the startup hint wording.
    // Every hint word must carry dim styling; plain, partially dim and blank
    // drafts retain their text. Never infer emptiness from grey color alone.
    if fully_dim {
        return Some(String::new());
    }
    if (agent == "codex" && input == "Ask Codex to do anything")
        || (agent == "claude" && input.starts_with("Try \"") && input.ends_with('"'))
    {
        return None;
    }
    Some(input.to_string())
}

/// Claude styles each suggestion word separately, so the spaces between words
/// carry no dim style. A hint is dim when it neither begins nor ends with
/// whitespace and every character is dim or an unstyled ASCII space.
fn dim_hint(hint: &str, dim: &[bool]) -> bool {
    let (Some(first), Some(last)) = (hint.chars().next(), hint.chars().next_back()) else {
        return false;
    };
    !first.is_whitespace()
        && !last.is_whitespace()
        && hint
            .char_indices()
            .all(|(index, ch)| dim[index] || ch == ' ')
}

/// Whether the observed `input` is exactly the text a send pasted. Claude's
/// padding is not input (see `exact_composer_input`), so the pasted text is
/// compared without its own trailing spaces.
pub(crate) fn composer_holds(agent: &str, input: &str, pasted: &str) -> bool {
    if agent == "claude" {
        input == pasted.trim_end_matches(' ')
    } else {
        input == pasted
    }
}

fn styled_text(pane: &str) -> Option<(String, Vec<bool>)> {
    let mut plain = String::new();
    let mut styles = Vec::new();
    let mut dim = false;
    let mut chars = pane.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next()? {
                '[' => {}
                ']' => {
                    hyperlink::consume(&mut chars)?;
                    continue;
                }
                _ => return None,
            }
            let mut parameters = String::new();
            loop {
                let next = chars.next()?;
                if next == 'm' {
                    break;
                }
                if !(next.is_ascii_digit() || next == ';') || parameters.len() >= 64 {
                    return None;
                }
                parameters.push(next);
            }
            let values: Vec<u16> = if parameters.is_empty() {
                vec![0]
            } else {
                parameters
                    .split(';')
                    .map(|value| {
                        if value.is_empty() {
                            Some(0)
                        } else {
                            value.parse().ok()
                        }
                    })
                    .collect::<Option<_>>()?
            };
            let mut i = 0;
            while i < values.len() {
                match values[i] {
                    0 => dim = false,
                    2 => dim = true,
                    22 => dim = false,
                    38 | 48 | 58 => {
                        let skip = match values.get(i + 1)? {
                            5 => 2,
                            2 => 4,
                            _ => return None,
                        };
                        if i + skip >= values.len() {
                            return None;
                        }
                        i += skip;
                    }
                    1..=107 => {}
                    _ => return None,
                }
                i += 1;
            }
        } else {
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                return None;
            }
            if ch.is_control() && ch != '\n' {
                return None;
            }
            plain.push(ch);
            styles.extend(std::iter::repeat_n(dim, ch.len_utf8()));
        }
    }
    Some((plain, styles))
}

mod hyperlink;

#[cfg(test)]
mod tests;

pub(crate) mod cleanup;
