//! Resolve delivery IDs from their owning local or federated store.

use serde_json::Value;

use crate::{CliContext, CliError};

use super::{authenticate_token, invalid, lock_journal, lock_registry, now_epoch, projection};

pub(crate) fn delivery(
    context: &CliContext,
    session: &str,
    token: &str,
    id: &str,
) -> Result<Value, CliError> {
    let (_, current) = authenticate_token(context, session, token)?;
    let locked = lock_journal(context)?;
    if let Some(item) = locked.registry.find(|item| {
        item.message_id == id
            && item.sender.principal() == session
            && item.sender.generation() == current
    }) {
        return Ok(projection(&item));
    }
    drop(locked);

    // Same-machine sends are admitted directly into the mailbox registry and
    // never create a federation journal entry. Resolve those accepted IDs from
    // the store that owns them after checking the remote outbox first.
    let mut mailbox = lock_registry(context)?;
    super::super::clean_expired(&mut mailbox.registry, now_epoch());
    if let Some(message) = mailbox.registry.messages.iter().find(|message| {
        message.message_id == id
            && message.sender_session_id == session
            && message.sender_incarnation == current
    }) {
        return serde_json::to_value(super::super::mailbox::metadata(message))
            .map_err(|_| invalid());
    }
    Err(CliError::data(
        "message-not-found",
        "delivery does not exist; verify the message ID and sender session, then retry the original send with the same idempotency key",
        None,
    ))
}
