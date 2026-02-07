use std::thread;
use std::time::{Duration, Instant};

use crate::error::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitOutcome {
    pub attempts: u32,
    pub elapsed_ms: u64,
}

pub fn sleep_ms(ms: u64) {
    if ms > 0 {
        thread::sleep(Duration::from_millis(ms));
    }
}

pub fn wait_until<F>(
    condition_name: &str,
    timeout_ms: u64,
    poll_ms: u64,
    mut check: F,
) -> Result<WaitOutcome, CliError>
where
    F: FnMut() -> Result<bool, CliError>,
{
    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms.max(1));
    let mut attempts = 0u32;

    loop {
        attempts = attempts.saturating_add(1);
        if check()? {
            return Ok(WaitOutcome {
                attempts,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }

        if Instant::now() >= deadline {
            return Err(CliError::runtime(format!(
                "timed out waiting for {condition_name} after {timeout_ms}ms"
            )));
        }

        sleep_ms(poll_ms.max(1));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::wait_until;

    #[test]
    fn wait_until_succeeds_before_timeout() {
        static ATTEMPTS: AtomicU32 = AtomicU32::new(0);
        ATTEMPTS.store(0, Ordering::SeqCst);

        let outcome = wait_until("ready", 200, 1, || {
            let n = ATTEMPTS.fetch_add(1, Ordering::SeqCst);
            Ok(n >= 2)
        })
        .expect("should succeed");

        assert!(outcome.attempts >= 3);
        assert!(outcome.elapsed_ms <= 200);
    }

    #[test]
    fn wait_until_errors_on_timeout() {
        let err = wait_until("never", 5, 1, || Ok(false)).expect_err("should timeout");
        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("timed out waiting"));
    }
}
