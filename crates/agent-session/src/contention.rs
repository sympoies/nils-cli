//! Bounded waits for transient lifecycle contention.
use std::time::{Duration, Instant};

use crate::{CliError, FailedLaunchCleanup};
use serde_json::{Map, Value, json};

pub(crate) const LIFECYCLE_WAIT: Duration = Duration::from_secs(6);

pub(crate) fn is_busy(error: &CliError) -> bool {
    matches!(
        error.code(),
        "coordination-lock-timeout" | "session-record-lock-timeout"
    )
}

/// Each attempt receives the remaining monotonic budget. Only lock contention
/// is replayable: storage, identity and authorization failures return unchanged.
pub(crate) fn retry<T>(
    timeout: Duration,
    mut attempt: impl FnMut(Duration) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let started = Instant::now();
    let mut backoff = Duration::from_millis(10);
    loop {
        let error = match attempt(timeout.saturating_sub(started.elapsed())) {
            Ok(result) => return Ok(result),
            Err(error) if is_busy(&error) => error,
            Err(error) => return Err(error),
        };
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(recovery(error, "retry-request", "not_admitted"));
        }
        std::thread::sleep(backoff.min(remaining));
        backoff = backoff.saturating_mul(2).min(Duration::from_millis(100));
    }
}

fn recovery(mut error: CliError, action: &str, outcome: &str) -> CliError {
    if is_busy(&error) {
        let details = error.details_mut().get_or_insert_with(|| json!({}));
        details["retryable"] = json!(true);
        details["next_action"] = json!(action);
        details["recovery"] = json!({ "kind": "lifecycle-contention", "outcome": outcome });
        let hint = match action {
            "retry-create" => "Retry session creation; failed-start cleanup is complete.",
            "retry-delete" => "Retry deletion for the same session.",
            "retry-archive" => "Retry archiving the same session.",
            _ => "Wait for contention to clear, then retry the same request.",
        };
        return error.with_hint(hint);
    }
    error
}

pub(crate) fn create_error(error: CliError) -> CliError {
    recovery(error, "retry-create", "not_created")
}

pub(crate) fn close_error(error: CliError, reason: crate::board::CloseReason) -> CliError {
    let action = match reason {
        crate::board::CloseReason::Deleted => "retry-delete",
        crate::board::CloseReason::Archived => "retry-archive",
    };
    recovery(error, action, "record_retained")
}

pub(crate) fn incomplete_create_cleanup(details: &mut Map<String, Value>) {
    if details.get("recovery").and_then(|value| value.get("kind"))
        == Some(&json!("lifecycle-contention"))
    {
        details.insert("retryable".into(), json!(false));
        details.insert("next_action".into(), json!("inspect-start-cleanup"));
        details.insert(
            "recovery".into(),
            json!({ "kind": "lifecycle-contention", "outcome": "cleanup_incomplete" }),
        );
    }
}

pub(crate) fn failed_launch_cleanup(mut error: CliError, cleanup: FailedLaunchCleanup) -> CliError {
    if cleanup == FailedLaunchCleanup::Completed {
        return create_error(error);
    }
    let mut details = match error.details_mut().take() {
        Some(Value::Object(details)) => details,
        _ => Map::new(),
    };
    incomplete_create_cleanup(&mut details);
    details.insert("cleanup".to_string(), cleanup.projection());
    *error.details_mut() = Some(Value::Object(details));
    if is_busy(&error) {
        error.with_hint("Inspect failed-start cleanup before creating another session.")
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn retries_only_proven_busy_failures_within_one_budget() {
        let mut attempts = 0;
        let started = Instant::now();
        let error = retry::<()>(Duration::from_millis(40), |remaining| {
            attempts += 1;
            assert!(remaining <= Duration::from_millis(40));
            Err(CliError::runtime("coordination-lock-timeout", "busy", None))
        })
        .unwrap_err();
        assert!(attempts > 1);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(error.details().unwrap()["retryable"], true);
        for code in [
            "coordination-store-untrusted",
            "coordination-store-unavailable",
            "coordination-store-corrupt",
            "codex-account-not-bound",
        ] {
            let mut attempts = 0;
            let error = retry::<()>(LIFECYCLE_WAIT, |_| {
                attempts += 1;
                Err(CliError::runtime(code, "permanent", None))
            })
            .unwrap_err();
            assert_eq!(attempts, 1);
            assert_eq!(error.code(), code);
            assert!(error.details().is_none());
        }
    }

    #[test]
    fn incomplete_start_cleanup_cannot_authorize_blind_create_retry() {
        let error = retry::<()>(Duration::ZERO, |_| {
            Err(CliError::runtime("coordination-lock-timeout", "busy", None))
        })
        .unwrap_err();
        let error =
            failed_launch_cleanup(error, FailedLaunchCleanup::Pending("process_boundary_live"));
        assert_eq!(error.details().unwrap()["retryable"], false);
        assert_eq!(
            error.details().unwrap()["next_action"],
            "inspect-start-cleanup"
        );
        assert_eq!(
            error.details().unwrap()["recovery"]["outcome"],
            "cleanup_incomplete"
        );
    }
}
