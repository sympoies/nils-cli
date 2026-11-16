//! Scoped budgets for synthetic transaction tests; absent from shipped builds.
use std::{cell::Cell, path::Path, time::Duration};

thread_local! {
    static CURRENT: Cell<Option<Duration>> = const { Cell::new(None) };
}

pub(crate) struct Scope(Option<Duration>);

impl Scope {
    pub(crate) fn enter(budget: Duration) -> Self {
        Self(CURRENT.replace(Some(budget)))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.set(self.0);
    }
}

pub(crate) fn current() -> Option<Duration> {
    CURRENT.get()
}

pub(super) fn capture_visible_pane_with_timeout(
    tmux: &Path,
    target: &str,
    timeout: Duration,
) -> Option<String> {
    crate::capture_visible_pane_with_timeout(tmux, target, current().unwrap_or(timeout))
}
