//! Scoped budgets for synthetic transaction tests; absent from shipped builds.
use std::{
    cell::Cell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::Duration,
};

static FIXTURES: LazyLock<Mutex<HashMap<PathBuf, Duration>>> = LazyLock::new(Default::default);

/// The fixture state path lets an HTTP handler recover the budget on its blocking worker.
pub(crate) struct Fixture(PathBuf);

impl Fixture {
    pub(crate) fn install(state_dir: &Path, budget: Duration) -> Self {
        let previous = FIXTURES
            .lock()
            .unwrap()
            .insert(state_dir.to_owned(), budget);
        assert!(previous.is_none());
        Self(state_dir.to_owned())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        FIXTURES.lock().unwrap().remove(&self.0);
    }
}

thread_local! {
    static CURRENT: Cell<Option<Duration>> = const { Cell::new(None) };
}

pub(crate) struct Scope(Option<Duration>);

impl Scope {
    pub(crate) fn for_fixture(state_dir: &Path) -> Option<Self> {
        let budget = FIXTURES.lock().unwrap().get(state_dir).copied();
        budget.map(Self::enter)
    }

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
