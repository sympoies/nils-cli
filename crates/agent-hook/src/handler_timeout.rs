//! Per-child deadline for dispatch handlers and coordination helpers.

use std::time::Duration;

/// Deadline every handler or helper child gets before the timeout posture applies.
const HANDLER_TIMEOUT: Duration = Duration::from_secs(5);
/// Shortens the deadline so timeout behavior can be tested without waiting out
/// the full budget. Whoever can set it can already set `PATH` and `HOME`, which
/// outrank the handler lookup; it can never lengthen the deadline.
const HANDLER_TIMEOUT_ENV: &str = "AGENT_HOOK_HANDLER_TIMEOUT_MS";

pub(crate) fn handler_timeout() -> Duration {
    shortened(std::env::var(HANDLER_TIMEOUT_ENV).ok().as_deref())
}

fn shortened(requested_ms: Option<&str>) -> Duration {
    requested_ms
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map_or(HANDLER_TIMEOUT, |millis| {
            Duration::from_millis(millis).min(HANDLER_TIMEOUT)
        })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn default_deadline_is_five_seconds() {
        assert_eq!(shortened(None), Duration::from_secs(5));
    }

    #[test]
    fn override_shortens_the_deadline() {
        assert_eq!(shortened(Some("750")), Duration::from_millis(750));
    }

    #[test]
    fn override_cannot_lengthen_the_deadline() {
        assert_eq!(shortened(Some("60000")), Duration::from_secs(5));
    }

    #[test]
    fn unusable_override_keeps_the_default() {
        for value in ["", "0", "-1", "soon", "1.5"] {
            assert_eq!(shortened(Some(value)), Duration::from_secs(5), "{value:?}");
        }
    }
}
