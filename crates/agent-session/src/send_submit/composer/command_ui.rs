//! Positive command UI anchors, separate from exact composer/input proof.
use super::styled_text;

pub(crate) fn observed(agent: &str, pane: &str) -> bool {
    if agent != "claude" {
        return false;
    }
    let Some((plain, _)) = styled_text(pane) else {
        return false;
    };
    let lines: Vec<&str> = plain.lines().collect();
    let Some(end) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return false;
    };
    if lines[end].trim() != "Esc to cancel" {
        return false;
    }
    // Claude's native Settings frame, observed with its Status tab. Body
    // values are irrelevant: dispatch proves neither settings nor login success.
    // Unknown dialogs and a composer below a historical frame remain unknown.
    (1..end).rev().any(|header| {
        lines[header]
            .split_whitespace()
            .eq(["Settings", "Status", "Config", "Usage", "Stats"])
            && lines[header - 1].trim().chars().count() >= 10
            && lines[header - 1].trim().chars().all(|ch| ch == '─')
            && !lines[header + 1..end]
                .iter()
                .any(|line| line.trim_start().starts_with(['❯', '›']))
    })
}
