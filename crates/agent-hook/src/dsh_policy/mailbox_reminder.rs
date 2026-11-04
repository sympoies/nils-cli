//! Validate the current mailbox wake envelope after recipient and schema checks.
use nils_common::session_binary::NAME;

const HEADER: &str = "<!-- agent-notice:v1 kind=mailbox.wake source=agent-session -->\n\n";
// Count summaries and generic verification guidance fit this bounded context.
const MAX_BYTES: usize = 4096;

pub(super) fn is_mailbox_notice(reminder: &str) -> bool {
    if reminder.len() > MAX_BYTES || reminder.chars().any(|ch| ch.is_control() && ch != '\n') {
        return false;
    }
    let Some(body) = reminder.strip_prefix(HEADER) else {
        return false;
    };
    !body.contains("<!-- agent-notice:") && body.lines().next().is_some_and(is_summary)
}

fn is_summary(line: &str) -> bool {
    let Some(summary) = line.strip_prefix("Mailbox: ") else {
        return false;
    };
    if is_inbox_action(summary) {
        return true;
    }
    let Some((counts, action)) = summary.split_once("; ") else {
        return false;
    };
    if !is_inbox_action(action) {
        return false;
    }
    if let Some(total) = counts.strip_suffix(" unread") {
        return canonical_count(total);
    }
    let categories = [
        "blocker",
        "decision",
        "handoff",
        "progress",
        "report",
        "uncategorized",
    ];
    let mut previous = None;
    !counts.is_empty()
        && counts.split(", ").all(|part| {
            let Some((count, category)) = part.split_once(' ') else {
                return false;
            };
            let Some(index) = categories.iter().position(|item| *item == category) else {
                return false;
            };
            if previous.is_some_and(|last| last >= index) || !canonical_count(count) {
                return false;
            }
            previous = Some(index);
            true
        })
}

fn canonical_count(count: &str) -> bool {
    count
        .parse::<u16>()
        .is_ok_and(|value| (1..=256).contains(&value) && value.to_string() == count)
}

fn is_inbox_action(action: &str) -> bool {
    action.strip_suffix(" message inbox") == Some(NAME)
}

#[cfg(test)]
#[path = "mailbox_notice_contract_tests.rs"]
mod notice_contract_tests;
