//! Wrapped rows may be matched for cleanup, never for submit admission.
use super::{claude_composer, codex_composer, exact_composer_input, styled_text};

pub(crate) fn input(agent: &str, pane: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::input;
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
