//! Wrapped-row matching for cleanup and cursor-verified submit admission.
use super::{claude_composer, codex_composer, exact_composer_input, styled_text};

/// The known single-line paste can disambiguate written padding at a wrap.
/// Matching alone is not submit admission or proof of unknown draft rows.
pub(crate) fn input(agent: &str, pane: &str, pasted: Option<&str>) -> Option<String> {
    if let Some(input) = exact_composer_input(agent, pane) {
        return Some(input);
    }
    if agent == "claude"
        && let Some(pasted) = pasted
    {
        // Joining arbitrary wrapped rows loses separator spaces. Without a
        // unique match, their content and counts are unobservable.
        return matched_layout(agent, pane, pasted).map(|(input, _)| input);
    }
    observed_input(agent, pane)
}

fn observed_input(agent: &str, pane: &str) -> Option<String> {
    if let Some(input) = exact_composer_input(agent, pane) {
        return Some(input);
    }
    let (plain, dim) = styled_text(pane)?;
    let lines: Vec<&str> = plain.lines().collect();
    let (region, _) = match agent {
        "claude" => claude_composer(&lines),
        "codex" => codex_composer(&lines),
        _ => None,
    }?;
    if region.len() < 2 {
        return None;
    }
    // At a wrap boundary, written Claude padding and a pasted space are
    // indistinguishable. Trimming there would invent a different input.
    if agent == "claude"
        && region[..region.len() - 1]
            .iter()
            .any(|row| row.ends_with(' '))
    {
        return None;
    }
    let start = lines
        .iter()
        .position(|line| std::ptr::eq(*line, region[0]))?;
    let offset = lines[..start]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>();
    let length = region.iter().map(|line| line.len() + 1).sum::<usize>() - 1;
    // A wrapped suggestion or partially styled draft is not proof of a paste.
    if dim[offset..offset + length].iter().any(|dim| *dim) {
        return None;
    }
    let first = if agent == "claude" {
        region[0].trim_end_matches(' ')
    } else {
        region[0]
    };
    let mut joined = first.to_string();
    for row in &region[1..] {
        let row = row.strip_prefix("  ")?;
        let row = if agent == "claude" {
            row.trim_end_matches(' ')
        } else {
            row
        };
        // Blank rows can be a user-added line break, not a wrapped payload.
        if row.is_empty() {
            return None;
        }
        joined.push_str(row);
    }
    let mut flattened = lines[..start].join("\n");
    flattened.push('\n');
    flattened.push_str(&joined);
    flattened.push('\n');
    flattened.push_str(&lines[start + region.len()..].join("\n"));
    // Retain the normal dialog, queue and paste-placeholder refusals. Joining
    // rows is cleanup-only: the caller still requires equality with its paste.
    exact_composer_input(agent, &flattened)
}

/// C-a must land at the beginning of the first composer row. This guards
/// against hidden hard newlines and unsupported/customized navigation keys.
pub(crate) fn cursor_at_start(pane: &str, x: usize, y: usize) -> bool {
    let Some((plain, _)) = styled_text(pane) else {
        return false;
    };
    let lines: Vec<&str> = plain.lines().collect();
    let Some((region, _)) = claude_composer(&lines) else {
        return false;
    };
    x == 2
        && lines
            .get(y)
            .is_some_and(|line| std::ptr::eq(*line, region[0]))
}

/// Expected visual endpoint of the known ASCII prefix. Moving exactly this
/// many characters and checking the cursor disambiguates invisible separators
/// before deleting: an extra/missing space must not consume a draft character.
pub(crate) fn prefix_end(pane: &str, pasted: &str) -> Option<(usize, usize)> {
    if pasted.is_empty() || !pasted.bytes().all(|b| (32..127).contains(&b)) {
        return None;
    }
    if exact_composer_input("claude", pane).is_some_and(|input| input.starts_with(pasted)) {
        let (plain, _) = styled_text(pane)?;
        let lines: Vec<&str> = plain.lines().collect();
        let (region, _) = claude_composer(&lines)?;
        let row = lines
            .iter()
            .position(|line| std::ptr::eq(*line, region[0]))?;
        return Some((2 + pasted.len(), row));
    }
    matched_layout("claude", pane, pasted).map(|(_, end)| end)
}

/// Match each visible row against the known paste, considering only ASCII
/// spaces at row ends as padding or omitted word separators. Retain every offset; more
/// than one reconstruction is uncertainty, not permission to delete.
fn matched_layout(agent: &str, pane: &str, pasted: &str) -> Option<(String, (usize, usize))> {
    if agent != "claude" || pasted.is_empty() || !pasted.bytes().all(|b| (32..127).contains(&b)) {
        return None;
    }
    let (plain, dim) = styled_text(pane)?;
    let lines: Vec<&str> = plain.lines().collect();
    let (region, _) = claude_composer(&lines)?;
    if region.len() < 2 {
        return None;
    }
    let start = lines
        .iter()
        .position(|line| std::ptr::eq(*line, region[0]))?;
    let offset = lines[..start]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>();
    let length = region.iter().map(|line| line.len() + 1).sum::<usize>() - 1;
    if dim[offset..offset + length].iter().any(|dim| *dim) {
        return None;
    }
    let first = region[0]
        .strip_prefix('❯')?
        .strip_prefix([' ', '\u{a0}', '\u{2007}', '\u{202f}'])?;
    let mut offsets = vec![0];
    let mut result = None;
    for (index, row) in region.iter().enumerate() {
        let row = if index == 0 {
            first
        } else {
            row.strip_prefix("  ")?
        };
        let visible = row.trim_end_matches(' ');
        if visible.is_empty() {
            return None;
        }
        let mut next = Vec::new();
        for position in offsets {
            let rest = &pasted[position..];
            if index + 1 == region.len() {
                // Unknown draft content must fit entirely in this last row.
                if let Some(suffix) = visible.strip_prefix(rest) {
                    let candidate = (format!("{pasted}{suffix}"), (2 + rest.len(), start + index));
                    if result
                        .as_ref()
                        .is_some_and(|previous| previous != &candidate)
                    {
                        return None;
                    }
                    result = Some(candidate);
                }
            } else if let Some(after) = rest.strip_prefix(visible) {
                let base = position + visible.len();
                let spaces = after.bytes().take_while(|b| *b == b' ').count();
                // Word wrapping can omit the separator space entirely.
                for count in 0..=spaces {
                    // Never infer unknown draft rows after the paste ends.
                    if base + count < pasted.len() {
                        next.push(base + count);
                    }
                }
            }
        }
        next.sort_unstable();
        next.dedup();
        offsets = next;
    }
    let (input, end) = result?;
    let flattened = format!(
        "{}\n❯ {}\n{}",
        lines[..start].join("\n"),
        input,
        lines[start + region.len()..].join("\n")
    );
    // Preserve the ordinary dialog and placeholder checks after matching.
    exact_composer_input(agent, &flattened)
        .filter(|observed| observed == &input)
        .map(|input| (input, end))
}

#[cfg(test)]
mod tests {
    use super::observed_input as input;
    use pretty_assertions::assert_eq;

    #[test]
    fn cleanup_rejects_blank_rows_styled_suggestions_and_dialogs() {
        for body in [
            "❯ pasted\n  ",
            "❯ \x1b[2mpasted\n  hint\x1b[22m",
            "❯ pasted\n  text\n  ",
        ] {
            let pane = format!("────────────────────\n{body}\n────────────────────\n  status");
            assert_eq!(input("claude", &pane), None);
        }
        let dialog = "› pasted\n  text\n\n  enter to confirm";
        assert_eq!(input("codex", dialog), None);
    }

    #[test]
    fn cleanup_keeps_codex_spaces_at_wrap_boundaries() {
        let pane = "› pasted \n  text\n\n  status";
        assert_eq!(input("codex", pane).as_deref(), Some("pasted text"));
    }
}
