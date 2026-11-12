//! Recognize Claude Code 2.1.317 opaque paste markers after empty admission.
use super::{DIALOG_HINTS, claude_composer, styled_text};

pub(crate) fn claude_paste_placeholder(pane: &str) -> Option<String> {
    let (plain, _) = styled_text(pane)?;
    let lines: Vec<_> = plain.lines().collect();
    let (region, footer) = claude_composer(&lines)?;
    if region.len() != 1
        || footer.iter().any(|line| {
            let lower = line.to_lowercase();
            DIALOG_HINTS.iter().any(|hint| lower.contains(hint))
        })
    {
        return None;
    }
    let after = region[0].trim_start().strip_prefix('❯')?;
    let input = after
        .strip_prefix([' ', '\u{a0}', '\u{2007}', '\u{202f}'])
        .unwrap_or(after)
        .trim_end_matches(' ');
    let body = input.strip_prefix("[Pasted text #")?.strip_suffix(']')?;
    let (number, lines) = body
        .split_once(" +")
        .map_or((body, None), |(number, count)| (number, Some(count)));
    let positive = |s: &str| {
        !s.is_empty()
            && s.bytes().all(|b| b.is_ascii_digit())
            && s.parse::<u32>().is_ok_and(|n| n > 0)
    };
    if !positive(number) || lines.is_some_and(|s| !s.strip_suffix(" lines").is_some_and(positive)) {
        return None;
    }
    Some(input.to_owned())
}

/// A mixed or wrapped marker is grounds for refusal, never paste ownership.
pub(crate) fn has_claude_paste_placeholder(pane: &str) -> bool {
    let Some((plain, _)) = styled_text(pane) else {
        return false;
    };
    let lines: Vec<_> = plain.lines().collect();
    let Some((region, _)) = claude_composer(&lines) else {
        return false;
    };
    region
        .iter()
        .any(|row| super::compact(row).contains("[Pastedtext#"))
}
