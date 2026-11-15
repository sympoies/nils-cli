//! Scoped logical time and lifecycle events for synthetic restart fixtures only.
use crate::CliContext;
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant as RealInstant},
};

type Action = Box<dyn FnOnce(&CliContext, &str) + Send>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    ExitRequested,
    Ready,
    ClearSubmitted,
}
#[derive(Default)]
struct State {
    elapsed: Duration,
    deadlines: HashMap<RealInstant, Instant>,
    actions: Vec<(Phase, String, Action)>,
    observations: Vec<Duration>,
}
struct Clock {
    base: RealInstant,
    state: Mutex<State>,
}
static CLOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Clock>>>> = LazyLock::new(Default::default);
thread_local! { static CURRENT: RefCell<Option<Arc<Clock>>> = const { RefCell::new(None) }; }

pub(super) struct Fixture(PathBuf);
pub(super) fn install(root: &Path) -> Fixture {
    let state_dir = root.join("state");
    assert!(
        CLOCKS
            .lock()
            .unwrap()
            .insert(
                state_dir.clone(),
                Arc::new(Clock {
                    base: RealInstant::now(),
                    state: Mutex::new(State::default()),
                })
            )
            .is_none()
    );
    Fixture(state_dir)
}
impl Drop for Fixture {
    fn drop(&mut self) {
        CLOCKS.lock().unwrap().remove(&self.0);
    }
}
fn clock(context: &CliContext) -> Option<Arc<Clock>> {
    CLOCKS.lock().unwrap().get(&context.state_dir).cloned()
}
pub(super) struct Scope(Option<Arc<Clock>>);
pub(super) fn enter(context: &CliContext) -> Scope {
    Scope(CURRENT.replace(clock(context)))
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.set(self.0.take());
    }
}

/// Unregistered tests retain std time. A registered fixture advances only at
/// sleeps or explicitly simulated observation timeouts, never while descheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Instant(RealInstant);
impl Instant {
    pub(super) fn now() -> Self {
        CURRENT.with_borrow(|clock| {
            Self(clock.as_ref().map_or_else(RealInstant::now, |clock| {
                clock.base + clock.state.lock().unwrap().elapsed
            }))
        })
    }
    pub(super) fn elapsed(self) -> Duration {
        Self::now().saturating_duration_since(self)
    }
    pub(super) fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_duration_since(earlier.0)
    }
}
impl std::ops::Add<Duration> for Instant {
    type Output = Self;
    fn add(self, duration: Duration) -> Self {
        Self(self.0 + duration)
    }
}
pub(super) fn admission_deadline(deadline: Instant) -> RealInstant {
    CURRENT.with_borrow(|clock| {
        let Some(clock) = clock else {
            return deadline.0;
        };
        // Admission still runs real fixture subprocesses. Retain the logical
        // deadline separately so their scheduling cannot spend its budget.
        let token = RealInstant::now() + Duration::from_secs(60);
        clock
            .state
            .lock()
            .unwrap()
            .deadlines
            .insert(token, deadline);
        token
    })
}
pub(crate) fn remaining(deadline: RealInstant) -> Option<Duration> {
    CURRENT.with_borrow(|clock| {
        let clock = clock.as_ref()?;
        let state = clock.state.lock().unwrap();
        let deadline = state.deadlines.get(&deadline)?;
        Some(
            deadline
                .0
                .saturating_duration_since(clock.base + state.elapsed),
        )
    })
}
pub(super) fn sleep(duration: Duration) {
    if !CURRENT.with_borrow(|clock| {
        if let Some(clock) = clock {
            clock.state.lock().unwrap().elapsed += duration;
            true
        } else {
            false
        }
    }) {
        std::thread::sleep(duration);
    }
}
pub(super) fn elapsed(context: &CliContext) -> Duration {
    clock(context).unwrap().state.lock().unwrap().elapsed
}
pub(super) fn observations(context: &CliContext) -> Vec<Duration> {
    clock(context)
        .unwrap()
        .state
        .lock()
        .unwrap()
        .observations
        .clone()
}

pub(super) struct Hook(Arc<AtomicBool>);
impl Hook {
    pub(super) fn join(self) -> Result<(), &'static str> {
        self.0
            .load(Ordering::SeqCst)
            .then_some(())
            .ok_or("lifecycle event was not reached")
    }
}
pub(super) fn on(
    context: &CliContext,
    phase: Phase,
    old_launch: &str,
    action: impl FnOnce(&CliContext, &str) + Send + 'static,
) -> Hook {
    let fired = Arc::new(AtomicBool::new(false));
    let done = fired.clone();
    clock(context).unwrap().state.lock().unwrap().actions.push((
        phase,
        old_launch.into(),
        Box::new(move |context, launch| {
            action(context, launch);
            done.store(true, Ordering::SeqCst);
        }),
    ));
    Hook(fired)
}
pub(crate) fn fire(context: &CliContext, phase: Phase, launch: &str) {
    let Some(clock) = clock(context) else {
        return;
    };
    let action = {
        let mut state = clock.state.lock().unwrap();
        let index = state.actions.iter().position(|(event, old, _)| {
            *event == phase && (phase != Phase::Ready || old != launch)
        });
        index.map(|index| state.actions.remove(index).2)
    };
    if let Some(action) = action {
        action(context, launch);
    }
}

/// Model a blocked fixture command at its observation boundary, consuming the
/// actual supplied timeout without starting a sleeping subprocess.
fn stalled(tmux: &Path, marker: &str, timeout: Duration) -> bool {
    CURRENT.with_borrow(|clock| {
        let Some(clock) = clock else {
            return false;
        };
        let Some(root) = tmux.parent() else {
            return false;
        };
        if !root.join(marker).exists() {
            return false;
        }
        let mut state = clock.state.lock().unwrap();
        state.observations.push(timeout);
        state.elapsed += timeout;
        if marker == "stall-status" {
            std::fs::write(root.join("status-stalled"), "").unwrap();
        }
        true
    })
}

// Receive the exact timeout at the production call boundary. Keeping the
// adapter below argument evaluation lets the readiness tests detect a missing
// cap or an oversized timeout in the caller.
pub(crate) fn live_status_with_timeout(tmux: &Path, session: &str, timeout: Duration) -> String {
    if stalled(tmux, "stall-status", timeout) {
        return "unknown".to_string();
    }
    crate::live_status_with_timeout(tmux, session, timeout)
}

pub(crate) fn tmux_environment_value(
    tmux: &Path,
    session: &str,
    name: &str,
    timeout: Duration,
) -> Result<String, crate::SessionTerminationFailure> {
    if stalled(tmux, "stall", timeout) {
        return Err(crate::SessionTerminationFailure::VerificationFailed);
    }
    if let Some(value) = environment(tmux, name) {
        return value.ok_or(crate::SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    crate::tmux_environment_value(tmux, session, name, timeout)
}

/// The synthetic composer is already stored as fixture bytes. Reading it in
/// process avoids racing a one-second shell capture against the stress load.
pub(crate) fn capture(tmux: &Path) -> Option<Option<String>> {
    let root = tmux.parent()?;
    if !CLOCKS.lock().unwrap().contains_key(&root.join("state")) {
        return None;
    }
    Some(
        root.join("running")
            .exists()
            .then(|| std::fs::read_to_string(root.join("screen")).ok())
            .flatten(),
    )
}

/// Preserve the fixture's actual runtime identity values without launching
/// three deadline-bound shells merely to read an already-materialized file.
fn environment(tmux: &Path, name: &str) -> Option<Option<String>> {
    let root = tmux.parent()?;
    if !CLOCKS.lock().unwrap().contains_key(&root.join("state")) {
        return None;
    }
    Some(if root.join("running").exists() {
        std::fs::read_to_string(root.join("env"))
            .ok()
            .and_then(|env| {
                env.lines()
                    .find_map(|line| line.strip_prefix(&format!("{name}=")).map(str::to_owned))
            })
    } else {
        None
    })
}
