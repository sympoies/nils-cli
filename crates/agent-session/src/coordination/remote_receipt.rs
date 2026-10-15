//! Rebuild terminal delivery receipts without losing admission metadata.
use serde_json::{Value, json};

use super::{DELIVERY_VERSION, Retained};

impl Retained {
    /// The receipt drain accepted: exactly these fields, checked against this
    /// message ID and recipient.
    pub(super) fn receipt(&self) -> Option<Value> {
        self.persisted_at_epoch.map(|persisted| {
            let mut receipt = json!({"schema_version":DELIVERY_VERSION,"message_id":self.message_id,"state":"delivered","recipient":self.recipient,"persisted_at_epoch":persisted});
            if let Some(suppressed) = self.notification_suppressed { receipt["notification_suppressed"] = json!(suppressed); }
            receipt
        })
    }
}
