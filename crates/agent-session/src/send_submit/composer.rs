//! Exact composer observation, retaining dim styling for native empty hints.
use super::{
    CLAUDE_QUEUED_HINT, DIALOG_HINTS, PASTE_PLACEHOLDERS, claude_composer, codex_composer, compact,
};

pub(crate) fn exact_composer_input(agent: &str, pane: &str) -> Option<String> {
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
    let fully_dim = dim_hint(input, &dim[offset..offset + input.len()]);
    let compressed = compact(input);
    if compressed == CLAUDE_QUEUED_HINT
        || PASTE_PLACEHOLDERS
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
