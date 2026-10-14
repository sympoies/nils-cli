//! Sticky schema fences for strict retained-journal readers.
use super::*;

pub(super) fn select(journal: &Journal) -> &'static str {
    let refusal = journal.schema_version == REFUSAL_JOURNAL_VERSION
        || journal
            .remote_outbox
            .iter()
            .any(|entry| entry.refusal_details.is_some())
        || journal
            .retained
            .iter()
            .any(|entry| entry.refusal_details.is_some());
    let audit = journal.schema_version == AUDIT_JOURNAL_VERSION
        || journal.remote_outbox.iter().any(|entry| {
            entry.last_attempt_at_epoch.is_some() || entry.state_changed_at_epoch.is_some()
        })
        || journal.retained.iter().any(|entry| {
            entry.created_at_epoch.is_some()
                || entry.last_attempt_at_epoch.is_some()
                || entry.state_changed_at_epoch.is_some()
        });
    let extended = journal.schema_version == EXTENDED_JOURNAL_VERSION
        || journal
            .remote_outbox
            .iter()
            .any(|entry| entry.envelope.category.is_some() || entry.envelope.forwarding.is_some())
        || journal
            .retained
            .iter()
            .any(|entry| entry.category.is_some() || entry.forwarding.is_some());
    if journal.schema_version == SUPPRESSION_JOURNAL_VERSION
        || journal
            .retained
            .iter()
            .any(|entry| entry.notification_suppressed.is_some())
        || journal.remote_outbox.iter().any(|entry| {
            entry
                .receipt
                .as_ref()
                .and_then(|receipt| receipt["notification_suppressed"].as_bool())
                .is_some()
        })
    {
        SUPPRESSION_JOURNAL_VERSION
    } else if refusal {
        REFUSAL_JOURNAL_VERSION
    } else if audit {
        AUDIT_JOURNAL_VERSION
    } else if extended {
        EXTENDED_JOURNAL_VERSION
    } else {
        JOURNAL_VERSION
    }
}
