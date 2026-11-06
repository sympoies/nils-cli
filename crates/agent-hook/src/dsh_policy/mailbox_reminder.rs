//! Validate the current mailbox wake envelope after recipient and schema checks.
use nils_common::mailbox_notice::{
    DISPATCHER_GUIDANCE, PROMPT_PATH_PREFIX, PROMPT_PATH_SUFFIX, VERIFY_GUIDANCE,
};
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
    match body.split_once('\n') {
        Some((summary, guidance)) => is_summary(summary) && is_guidance(guidance),
        None => is_summary(body),
    }
}

fn is_guidance(body: &str) -> bool {
    let mut remaining = body;
    let mut relation_count = 0;
    let mut dispatching = false;
    for relation in ["parent", "ancestor", "peer"] {
        let label = format!("sender_relation={relation}\n");
        if let Some(rest) = remaining.strip_prefix(&label) {
            remaining = rest;
            relation_count += 1;
            dispatching |= relation != "peer";
        }
    }
    if relation_count == 0 {
        return false;
    }
    let Some(rest) = remaining.strip_prefix(VERIFY_GUIDANCE) else {
        return false;
    };
    if !dispatching {
        return rest.is_empty();
    }
    let Some(suffix) = rest
        .strip_prefix('\n')
        .and_then(|rest| rest.strip_prefix(DISPATCHER_GUIDANCE))
    else {
        return false;
    };
    if suffix == "." {
        return true;
    }
    suffix
        .strip_prefix(PROMPT_PATH_PREFIX)
        .and_then(|path| path.strip_suffix('.'))
        .and_then(|path| path.strip_suffix(PROMPT_PATH_SUFFIX))
        .is_some_and(|path| !path.is_empty() && !path.contains('\n'))
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
