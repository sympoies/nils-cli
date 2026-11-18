//! Polling acquisitions share the waiter's original budget, including retries.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::coordination::{LockedRegistry, RegistryMaintenance, lock_holder, lock_registry_within};
use crate::{CliContext, CliError};

pub(in crate::coordination) fn lock_for_wait(
    context: &CliContext,
    started: Instant,
    timeout: Duration,
    cancelled: Option<&AtomicBool>,
    timeout_error: fn() -> CliError,
) -> Result<LockedRegistry, CliError> {
    let mut backoff = Duration::from_millis(10);
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CliError::runtime(
                "wait-cancelled",
                "message wait was cancelled",
                None,
            ));
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(timeout_error());
        }
        match lock_registry_within(
            context,
            RegistryMaintenance::Full,
            lock_holder::HolderClass::FullMaintenance,
            remaining.min(super::CANCELLATION_CHECK_INTERVAL),
        ) {
            Ok(locked) => {
                if started.elapsed() >= timeout {
                    return Err(timeout_error());
                }
                return Ok(locked);
            }
            Err(error) if error.code() == "coordination-lock-timeout" => {}
            Err(error) => return Err(error),
        }
        // A notification may have been consumed before contention. Retry the
        // acquisition directly; another registry event is not required.
        std::thread::sleep(backoff.min(timeout.saturating_sub(started.elapsed())));
        backoff = backoff.saturating_mul(2).min(Duration::from_millis(100));
    }
}
