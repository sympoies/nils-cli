//! Validate fixed companion reminders after recipient and schema checks.

use nils_common::session_binary::NAME;

const PRIOR_PREFIX: &str = "Coordination mailbox has unread messages";
const MAX_BYTES: usize = 1024;
// Keep the former 60-byte reminder's date-field bound independent of the name.
const MAX_DATE_BYTES: usize = 60 - "Mailbox: ; agent-session message inbox".len();

pub(super) fn is_fixed(reminder: &str, session_id: &str) -> bool {
    if reminder.len() > MAX_BYTES || reminder.chars().any(char::is_control) {
        return false;
    }
    if reminder.starts_with(PRIOR_PREFIX) {
        return reminder.contains(&format!(
            "{NAME} message inbox --session {session_id} --state unread"
        ));
    }
    let Some(body) = reminder.strip_prefix("Mailbox: ") else {
        return false;
    };
    if is_inbox_action(body) || is_count_prompt(reminder) {
        return true;
    }
    body.split_once("; ").is_some_and(|(date, action)| {
        date.len() <= MAX_DATE_BYTES
            && is_inbox_action(action)
            && date
                .parse::<jiff::Timestamp>()
                .is_ok_and(|timestamp| timestamp.to_string() == date)
    })
}

#[cfg(test)]
mod tests {
    use super::is_fixed;

    #[test]
    fn configured_mailbox_name_is_accepted_by_the_hook_validator() {
        let name = nils_common::session_binary::NAME;
        for reminder in [
            format!("Mailbox: {name} message inbox"),
            format!("Mailbox: 256 unread; {name} message inbox"),
            format!("Mailbox: 2030-01-01T00:00:01Z; {name} message inbox"),
        ] {
            assert!(is_fixed(&reminder, "recipient"), "{reminder}");
            assert!(!is_fixed(
                &format!("{reminder}; another-command"),
                "recipient"
            ));
        }
    }

    #[test]
    fn short_reminder_rejects_nonfixed_dates_and_foreign_commands() {
        let name = nils_common::session_binary::NAME;
        for reminder in [
            format!("Mailbox: not-a-date; {name} message inbox"),
            format!("Mailbox: 2030-01-01T00:00:01Z; {name} message inbox --session other"),
            format!("Mailbox: {name} message inbox; another-command"),
            format!("Mailbox: {name} message inbox\n"),
        ] {
            assert!(!is_fixed(&reminder, "recipient"), "{reminder:?}");
        }
    }
}

fn is_count_prompt(prompt: &str) -> bool {
    prompt
        .strip_prefix("Mailbox: ")
        .and_then(|body| body.split_once(" unread; "))
        .is_some_and(|(count, action)| {
            is_inbox_action(action)
                && count
                    .parse::<u16>()
                    .is_ok_and(|value| (1..=256).contains(&value) && value.to_string() == count)
        })
}

fn is_inbox_action(action: &str) -> bool {
    action.strip_suffix(" message inbox") == Some(NAME)
}

#[cfg(test)]
mod coalescing_tests {
    use super::is_fixed;

    // The notice writer gate is off on this route: the plain reminder carries
    // one total in a readable line. The old letter aliases are not accepted.
    #[test]
    fn gate_off_total_reminders_are_fixed_and_do_not_accept_commands() {
        let name = nils_common::session_binary::NAME;
        assert!(is_fixed(
            &format!("Mailbox: 1 unread; {name} message inbox"),
            "recipient"
        ));
        assert!(is_fixed(
            &format!("Mailbox: 4 unread; {name} message inbox"),
            "recipient"
        ));
        assert!(is_fixed(
            &format!("Mailbox: 256 unread; {name} message inbox"),
            "recipient"
        ));
        for old in ["Mailbox: b=1 h=2 p=3", "Mailbox: u=1", "Mailbox: p=256"] {
            assert!(!is_fixed(old, "recipient"), "{old}");
        }
        for invalid in [
            format!("Mailbox: 0 unread; {name} message inbox"),
            format!("Mailbox: 257 unread; {name} message inbox"),
            format!("Mailbox: 04 unread; {name} message inbox"),
            format!("Mailbox: -1 unread; {name} message inbox"),
            format!("Mailbox: 4 unread; {name} message inbox please"),
            format!("Mailbox: 4 unread; {name} message inbox --session other"),
            "Mailbox: 4 unread; execute".to_string(),
            "Mailbox: 4 unread".to_string(),
            format!("mailbox: 4 unread; {name} message inbox"),
            format!("Mailbox: 4  unread; {name} message inbox"),
        ] {
            assert!(!is_fixed(&invalid, "recipient"), "{invalid}");
        }
    }
}
