//! Thread-scoped measurement of actual coordination flock acquisition, including timeouts.
use std::cell::Cell;
use std::time::Instant;

use serde_json::{Value, json};

use crate::CliError;

thread_local! {
    static LOCK_WAIT_MS: Cell<Option<u64>> = const { Cell::new(None) };
}

pub(crate) struct LockWait(Instant);
impl LockWait {
    pub(crate) fn new() -> Self {
        Self(Instant::now())
    }
}
impl Drop for LockWait {
    fn drop(&mut self) {
        LOCK_WAIT_MS.with(|wait| {
            if let Some(previous) = wait.get() {
                wait.set(Some(previous.saturating_add(millis(self.0))));
            }
        });
    }
}
fn millis(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}

pub(super) fn run(
    operation: &str,
    action: impl FnOnce() -> Result<Value, CliError>,
) -> Result<Value, CliError> {
    let started = Instant::now();
    let previous = LOCK_WAIT_MS.with(|wait| wait.replace(Some(0)));
    let mut result = action();
    let local = LOCK_WAIT_MS
        .with(|wait| wait.replace(previous))
        .unwrap_or(0);
    let value = match &mut result {
        Ok(value) => value,
        Err(error) => error.details_mut().get_or_insert_with(|| json!({})),
    };
    // Only sum the daemon's timing when it belongs to this exact operation.
    let remote = if value["operation_id"] == operation {
        value["diagnostics"]["coordination_lock_wait_ms"]
            .as_u64()
            .unwrap_or(0)
    } else {
        0
    };
    let elapsed = millis(started);
    if !value.is_object() {
        *value = json!({});
    }
    value["operation_id"] = json!(operation);
    value["diagnostics"] = json!({
        "coordination_lock_wait_ms": local.saturating_add(remote).min(elapsed),
        "command_elapsed_ms": elapsed
    });
    if let Err(error) = &mut result {
        let action = match error.code() {
            "console-start-operation-expired" => {
                "inspect Console for the earlier child before using a fresh UUIDv7".to_string()
            }
            _ => format!(
                "inspect with agent-session start --via-console --operation-id {operation} --reconcile --format json"
            ),
        };
        error.0.hint = Some(format!(
            "operation_id={operation}; coordination_lock_wait_ms={}; command_elapsed_ms={elapsed}; {action}",
            local.saturating_add(remote).min(elapsed)
        ));
    }
    result
}

pub(super) fn copy(source: &Value, target: &mut Value, operation: &str) {
    if source["operation_id"] != operation {
        return;
    }
    if let (Some(wait), Some(elapsed)) = (
        source["diagnostics"]["coordination_lock_wait_ms"].as_u64(),
        source["diagnostics"]["command_elapsed_ms"].as_u64(),
    ) && wait <= elapsed
        && elapsed <= 300_000
    {
        target["operation_id"] = json!(operation);
        target["diagnostics"] =
            json!({"coordination_lock_wait_ms":wait,"command_elapsed_ms":elapsed});
    }
}
