use std::thread;
use std::time::Duration;

use crate::error::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub retries: u8,
    pub retry_delay_ms: u64,
}

pub fn run_with_retry<T, F>(policy: RetryPolicy, mut op: F) -> Result<(T, u8), CliError>
where
    F: FnMut() -> Result<T, CliError>,
{
    let mut attempt = 0u8;
    loop {
        attempt = attempt.saturating_add(1);
        match op() {
            Ok(value) => return Ok((value, attempt)),
            Err(err) => {
                if err.exit_code() != 1 || attempt > policy.retries {
                    return Err(err);
                }
                if policy.retry_delay_ms > 0 {
                    thread::sleep(Duration::from_millis(policy.retry_delay_ms));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU8, Ordering};

    use pretty_assertions::assert_eq;

    use super::{run_with_retry, RetryPolicy};
    use crate::error::CliError;

    #[test]
    fn retries_runtime_errors_until_success() {
        static CALLS: AtomicU8 = AtomicU8::new(0);
        CALLS.store(0, Ordering::SeqCst);

        let policy = RetryPolicy {
            retries: 2,
            retry_delay_ms: 0,
        };
        let (value, attempts) = run_with_retry(policy, || {
            let n = CALLS.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(CliError::runtime("transient"))
            } else {
                Ok("ok")
            }
        })
        .expect("retry should eventually succeed");

        assert_eq!(value, "ok");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn does_not_retry_usage_errors() {
        let policy = RetryPolicy {
            retries: 3,
            retry_delay_ms: 0,
        };

        let err = run_with_retry::<(), _>(policy, || Err(CliError::usage("bad args")))
            .expect_err("usage errors must not be retried");

        assert_eq!(err.exit_code(), 2);
    }
}
