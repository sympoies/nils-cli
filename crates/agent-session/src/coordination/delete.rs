//! Conditional deletion's mailbox predicate. The caller must hold the recipient
//! session record lock until removal: both local and remote delivery acquire
//! that lock before the registry lock and revalidate the recipient afterwards.

use crate::{CliContext, CliError};
use serde_json::json;

pub(crate) fn ensure_unread_empty(context: &CliContext, id: &str) -> Result<(), CliError> {
    // Never use the public list snapshot: it deliberately defaults to zero
    // when the registry cannot be read. This read must prove the predicate.
    let locked = super::lock_registry_observational(context).map_err(|_| {
        CliError::runtime(
            "session-mailbox-unknown",
            "session unread mailbox state could not be verified",
            Some(json!({
                "predicate": "unread-empty",
                "retryable": true,
                "next_action": "inspect_mailbox_store_then_retry",
                "recovery": {"kind": "retry_conditional_delete"},
            })),
        )
    })?;
    let unread = locked
        .registry
        .messages
        .iter()
        .filter(|message| message.recipient_session_id == id && message.state == "unread")
        .count();
    if unread != 0 {
        return Err(CliError::data(
            "session-mailbox-not-empty",
            "session has unread messages; session state was retained",
            Some(json!({
                "predicate": "unread-empty",
                "unread_message_count": unread,
                "retryable": false,
                "next_action": "inspect_unread_messages",
                "recovery": {"kind": "review_mailbox"},
            })),
        ));
    }
    // Releasing the registry lock is safe: the caller still holds the lifecycle
    // fence through revocation and removal, so no new delivery can be admitted.
    Ok(())
}
