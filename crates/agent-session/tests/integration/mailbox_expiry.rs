//! Deterministic message expiry for integration tests.

use std::path::Path;

use serde_json::json;

use super::coordination::rewrite_registry;

/// Expires a stored message without waiting for the registry's epoch-second
/// clock to pass its deadline.
pub(super) fn expire_message(state_dir: &Path, message_id: &str) {
    rewrite_registry(state_dir, |registry| {
        let message = registry["messages"]
            .as_array_mut()
            .expect("message list")
            .iter_mut()
            .find(|message| message["message_id"] == message_id)
            .expect("stored message");
        message["expires_at_epoch"] = json!(1);
    });
}
