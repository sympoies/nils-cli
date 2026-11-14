//! Owned permission-hook timeout and handler identity for setup reconciliation.
use super::*;

/// Claude permission relay expires at 540s, before this provider hook deadline.
pub fn dispatch_timeout_seconds(product: Product, event: &str) -> i64 {
    if product == Product::Claude && event == "PermissionRequest" {
        600
    } else {
        DISPATCH_TIMEOUT_SECONDS
    }
}

pub(super) fn owned_json_handler(handler: &Value, product: Product) -> bool {
    handler.as_object().is_some_and(|object| object.len() == 3)
        && handler.get("type").and_then(Value::as_str) == Some("command")
        && handler.get("command").and_then(Value::as_str)
            == Some(dispatch_command(product).as_str())
        && handler.get("timeout").and_then(Value::as_i64).is_some()
}

pub(super) fn stale_timeout(handler: &Value, product: Product, event: &str) -> bool {
    handler.get("timeout").and_then(Value::as_i64) != Some(dispatch_timeout_seconds(product, event))
}
