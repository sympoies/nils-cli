//! Logical time for a lifecycle request holding real fixture locks.

use std::{
    cell::Cell,
    time::{Duration, Instant as RealInstant},
};

thread_local! {
    static NOW: Cell<Option<RealInstant>> = const { Cell::new(None) };
}

pub(crate) struct Clock(Instant);

impl Clock {
    pub(crate) fn start() -> Self {
        let start = RealInstant::now();
        assert!(NOW.with(Cell::get).is_none());
        NOW.set(Some(start));
        Self(Instant(start))
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        NOW.set(None);
    }
}

pub(crate) struct Instant(RealInstant);

impl Instant {
    pub(crate) fn now() -> Self {
        Self(NOW.with(Cell::get).unwrap_or_else(RealInstant::now))
    }

    pub(crate) fn elapsed(&self) -> Duration {
        Self::now().0.saturating_duration_since(self.0)
    }
}

pub(crate) fn sleep(duration: Duration) {
    if let Some(now) = NOW.with(Cell::get) {
        NOW.set(Some(now + duration));
    } else {
        std::thread::sleep(duration);
    }
}
