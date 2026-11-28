use jiff::{SignedDuration, Timestamp};
use std::cell::Cell;
use std::time::Duration;

thread_local! {
    static FROZEN: Cell<Option<Timestamp>> = const { Cell::new(None) };
}

/// The frozen activity clock for the current test thread, if one is installed.
pub(in crate::activity) fn frozen_now() -> Option<Timestamp> {
    FROZEN.with(Cell::get)
}

/// Freezes the activity clock on this thread until dropped, so a test crosses
/// a quiet window by advancing the clock instead of sleeping through it.
pub(super) struct TestClock;

impl TestClock {
    pub(super) fn freeze() -> Self {
        FROZEN.with(|frozen| frozen.set(Some(Timestamp::now())));
        Self
    }

    /// Freezes the clock at an instant the test owns, so elapsed time is measured
    /// from it instead of from whenever the host happened to schedule the test.
    pub(super) fn freeze_at(at: &str) -> Self {
        let at = at.parse::<Timestamp>().expect("RFC 3339 instant");
        FROZEN.with(|frozen| frozen.set(Some(at)));
        Self
    }

    pub(super) fn advance(&self, by: Duration) {
        FROZEN.with(|frozen| {
            let current = frozen.get().expect("frozen clock");
            let by = SignedDuration::try_from(by).expect("advance fits a signed duration");
            frozen.set(Some(current.checked_add(by).expect("advanced clock")));
        });
    }
}

impl Drop for TestClock {
    fn drop(&mut self) {
        FROZEN.with(|frozen| frozen.set(None));
    }
}
