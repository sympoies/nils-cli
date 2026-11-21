//! Authentication and polling acquisitions share the waiter's original budget.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::coordination::{
    LockedRegistry, RegistryMaintenance, capability, capability_token_from_file, lock_holder,
    lock_registry_within,
};
use crate::{CliContext, CliError, SessionRecord};

pub(in crate::coordination) struct WaitBudget<'a> {
    started: Instant,
    timeout: Duration,
    cancelled: Option<&'a AtomicBool>,
    timeout_error: fn() -> CliError,
}

impl<'a> WaitBudget<'a> {
    pub(in crate::coordination) fn new(
        timeout: Duration,
        cancelled: Option<&'a AtomicBool>,
        timeout_error: fn() -> CliError,
    ) -> Self {
        Self {
            started: Instant::now(),
            timeout,
            cancelled,
            timeout_error,
        }
    }

    pub(in crate::coordination) fn remaining(&self) -> Duration {
        self.timeout.saturating_sub(self.started.elapsed())
    }

    pub(in crate::coordination) fn authenticate(
        &self,
        context: &CliContext,
        session_id: &str,
        capability_file: &Path,
    ) -> Result<(SessionRecord, String), CliError> {
        let token = capability_token_from_file(Some(capability_file))?;
        capability::authenticate_token_with_lock(context, session_id, &token, || {
            self.lock_registry(context)
        })
    }

    pub(in crate::coordination) fn lock_registry(
        &self,
        context: &CliContext,
    ) -> Result<LockedRegistry, CliError> {
        let mut backoff = Duration::from_millis(10);
        loop {
            if self
                .cancelled
                .is_some_and(|flag| flag.load(Ordering::Acquire))
            {
                return Err(CliError::runtime(
                    "wait-cancelled",
                    "message wait was cancelled",
                    None,
                ));
            }
            let remaining = self.remaining();
            if remaining.is_zero() {
                return Err((self.timeout_error)());
            }
            match lock_registry_within(
                context,
                RegistryMaintenance::Full,
                lock_holder::HolderClass::FullMaintenance,
                remaining.min(super::CANCELLATION_CHECK_INTERVAL),
            ) {
                Ok(locked) => {
                    if self.remaining().is_zero() {
                        return Err((self.timeout_error)());
                    }
                    return Ok(locked);
                }
                Err(error) if error.code() == "coordination-lock-timeout" => {}
                Err(error) => return Err(error),
            }
            // Retry directly: polling may already have consumed the notification.
            std::thread::sleep(backoff.min(self.remaining()));
            backoff = backoff.saturating_mul(2).min(Duration::from_millis(100));
        }
    }
}
