//! Unit-only control for the proxy's real manual-cancellation lock attempts.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::ManualInputCancelOutcome;
use crate::{CliContext, SessionRecordLock};

type Key = (PathBuf, String);
type Registry = Mutex<HashMap<Key, Arc<Control>>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

pub(crate) struct Control {
    exhaust: bool,
    state: Mutex<State>,
}

struct State {
    lock: Option<SessionRecordLock>,
    attempts: usize,
    busy: usize,
}

impl Control {
    pub(super) fn budget(&self) -> Duration {
        // Exhaustion still performs a real failed lock attempt. Success has
        // scheduling allowance, but releases only after observing real Busy.
        if self.exhaust {
            Duration::ZERO
        } else {
            Duration::from_secs(120)
        }
    }

    pub(super) fn observe(&self, outcome: ManualInputCancelOutcome) {
        let mut state = self.state.lock().unwrap();
        state.attempts += 1;
        if outcome == ManualInputCancelOutcome::Busy {
            state.busy += 1;
            if !self.exhaust {
                state.lock.take();
            }
        }
    }
}

pub(super) fn for_session(context: &CliContext, id: &str) -> Option<Arc<Control>> {
    registry()
        .lock()
        .unwrap()
        .get(&(context.state_dir.clone(), id.to_string()))
        .cloned()
}

/// Removes the exact fixture's override and held lock even when a test panics.
pub(crate) struct Scope {
    key: Key,
    control: Arc<Control>,
}

impl Scope {
    pub(crate) fn new(
        context: &CliContext,
        id: &str,
        lock: SessionRecordLock,
        exhaust: bool,
    ) -> Self {
        let key = (context.state_dir.clone(), id.to_string());
        let control = Arc::new(Control {
            exhaust,
            state: Mutex::new(State {
                lock: Some(lock),
                attempts: 0,
                busy: 0,
            }),
        });
        assert!(
            registry()
                .lock()
                .unwrap()
                .insert(key.clone(), control.clone())
                .is_none()
        );
        Self { key, control }
    }

    pub(crate) fn counts(&self) -> (usize, usize) {
        let state = self.control.state.lock().unwrap();
        (state.attempts, state.busy)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        registry().lock().unwrap().remove(&self.key);
        self.control.state.lock().unwrap().lock.take();
    }
}
