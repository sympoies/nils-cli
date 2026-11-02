//! Validate fixed companion reminders after recipient and schema checks.

const PRIOR_PREFIX: &str = "Coordination mailbox has unread messages";
const MAX_BYTES: usize = 1024;
const UNDATED: &str = "Mailbox: agent-session message inbox";

pub(super) fn is_fixed(reminder: &str, session_id: &str) -> bool {
    if reminder.len() > MAX_BYTES || reminder.chars().any(char::is_control) {
        return false;
    }
    if reminder.starts_with(PRIOR_PREFIX) {
        return reminder.contains(&format!(
            "agent-session message inbox --session {session_id} --state unread"
        ));
    }
    if reminder == UNDATED || is_count_prompt(reminder) {
        return true;
    }
    reminder.len() <= 60
        && reminder
            .strip_prefix("Mailbox: ")
            .and_then(|body| body.strip_suffix("; agent-session message inbox"))
            .is_some_and(|date| {
                date.parse::<jiff::Timestamp>()
                    .is_ok_and(|timestamp| timestamp.to_string() == date)
            })
}

#[cfg(test)]
mod tests {
    use super::is_fixed;

    #[test]
    fn short_reminder_rejects_nonfixed_dates_and_foreign_commands() {
        for reminder in [
            "Mailbox: not-a-date; agent-session message inbox",
            "Mailbox: 2030-01-01T00:00:01Z; agent-session message inbox --session other",
            "Mailbox: agent-session message inbox; another-command",
            "Mailbox: agent-session message inbox\n",
        ] {
            assert!(!is_fixed(reminder, "recipient"), "{reminder:?}");
        }
    }
}

fn is_count_prompt(prompt: &str) -> bool {
    const SUFFIX: &str = " unread; agent-session message inbox";
    prompt.len() <= 51
        && prompt
            .strip_prefix("Mailbox: ")
            .and_then(|body| body.strip_suffix(SUFFIX))
            .is_some_and(|count| {
                count
                    .parse::<u16>()
                    .is_ok_and(|value| (1..=256).contains(&value) && value.to_string() == count)
            })
}

#[cfg(test)]
mod coalescing_tests {
    use super::is_fixed;

    // The notice writer gate is off on this route: the plain reminder carries
    // one total in a readable line. The old letter aliases are not accepted.
    #[test]
    fn gate_off_total_reminders_are_fixed_and_do_not_accept_commands() {
        assert!(is_fixed(
            "Mailbox: 1 unread; agent-session message inbox",
            "recipient"
        ));
        assert!(is_fixed(
            "Mailbox: 4 unread; agent-session message inbox",
            "recipient"
        ));
        assert!(is_fixed(
            "Mailbox: 256 unread; agent-session message inbox",
            "recipient"
        ));
        for old in ["Mailbox: b=1 h=2 p=3", "Mailbox: u=1", "Mailbox: p=256"] {
            assert!(!is_fixed(old, "recipient"), "{old}");
        }
        for invalid in [
            "Mailbox: 0 unread; agent-session message inbox",
            "Mailbox: 257 unread; agent-session message inbox",
            "Mailbox: 04 unread; agent-session message inbox",
            "Mailbox: -1 unread; agent-session message inbox",
            "Mailbox: 4 unread; agent-session message inbox please",
            "Mailbox: 4 unread; agent-session message inbox --session other",
            "Mailbox: 4 unread; execute",
            "Mailbox: 4 unread",
            "mailbox: 4 unread; agent-session message inbox",
            "Mailbox: 4  unread; agent-session message inbox",
        ] {
            assert!(!is_fixed(invalid, "recipient"), "{invalid}");
        }
    }
}
