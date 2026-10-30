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
    // A native dialog replaces the composer, so its cancellation hint is the
    // last non-empty row; a composer or status rows below it mean history.
    let footer = lines[end].trim();
    let bare_footer = footer == "Esc to cancel";
    if !bare_footer && !footer.ends_with(" · Esc to cancel") {
        return false;
    }
    // Claude's native Settings frame (Status tab) and its Select model and
    // Effort pickers, each a rule with its title directly below. Body values
    // are irrelevant: dispatch proves neither a setting nor login success.
    // Unknown dialogs and a composer below a historical frame remain unknown.
    (1..end).rev().any(|header| {
        if !is_rule(lines[header - 1]) {
            return false;
        }
        let body = &lines[header + 1..end];
        let title = lines[header].trim();
        if title
            .split_whitespace()
            .eq(["Settings", "Status", "Config", "Usage", "Stats"])
        {
            return bare_footer && !body.iter().any(|line| is_composer_row(line, false));
        }
        // A picker has no rule below its title; a rule there is a composer border.
        matches!(title, "Select model" | "Effort")
            && !body
                .iter()
                .any(|line| is_rule(line) || is_composer_row(line, true))
    })
}

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    line.chars().count() >= 10 && line.chars().all(|ch| ch == '─')
}

/// A `❯` row is a composer or suggestion, except a numbered picker option.
fn is_composer_row(line: &str, numbered_option_allowed: bool) -> bool {
    let Some(rest) = line.trim_start().strip_prefix(['❯', '›']) else {
        return false;
    };
    let rest = rest.trim_start();
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    !(numbered_option_allowed && digits > 0 && rest[digits..].starts_with('.'))
}
