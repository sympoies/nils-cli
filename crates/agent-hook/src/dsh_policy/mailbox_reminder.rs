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
    if reminder == UNDATED {
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
