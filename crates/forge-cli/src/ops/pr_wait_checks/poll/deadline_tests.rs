use super::*;
use crate::provider::DetectionSource;
use pretty_assertions::assert_eq;
use std::cell::Cell;

struct TestClock {
    start: Instant,
    elapsed: Cell<Duration>,
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.start + self.elapsed.get()
    }

    fn sleep(&self, duration: Duration) {
        self.elapsed.set(self.elapsed.get() + duration);
    }
}

struct InterruptedRead<'a> {
    clock: &'a TestClock,
    completed_polls: Cell<usize>,
    has_check: bool,
    interrupt_at: Duration,
}

impl BackendRunner for InterruptedRead<'_> {
    fn run(&self, _call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
        let remaining = self.completed_polls.get();
        if remaining == 0 {
            self.clock.elapsed.set(self.interrupt_at);
            return Err(ForgeError::unavailable(
                pr_checks::schema_err(),
                "backend_timeout",
                "injected snapshot timeout",
                None,
            ));
        }
        self.completed_polls.set(remaining - 1);
        // A numeric GitLab MR with no pipeline is a complete empty snapshot.
        // A running pipeline without a jobs endpoint is a pending check.
        let pipeline = if self.has_check {
            r#", "head_pipeline":{"status":"running"}"#
        } else {
            ""
        };
        Ok(BackendSuccess {
            stdout: format!(
                r#"{{"iid":1,"web_url":"https://gitlab.com/example/project/-/merge_requests/1"{pipeline}}}"#
            ),
            stderr: String::new(),
        })
    }
}

fn interrupted_wait(
    completed_polls: usize,
    has_check: bool,
    interrupt_at: Duration,
) -> WaitOutcome {
    let clock = TestClock {
        start: Instant::now(),
        elapsed: Cell::new(Duration::ZERO),
    };
    let runner = InterruptedRead {
        clock: &clock,
        completed_polls: Cell::new(completed_polls),
        has_check,
        interrupt_at,
    };
    let global = GlobalFlags {
        format: None,
        remote: "origin".into(),
        provider: None,
        host: None,
        repo: Some("example/project".into()),
        store_root: None,
        dry_run: false,
    };
    let ctx = ProviderContext {
        provider: Provider::GitLab,
        host: "gitlab.com".into(),
        source: DetectionSource::Flag,
        repo: global.repo.clone(),
    };
    let args = PrWaitChecksArgs {
        id: "1".into(),
        timeout: Duration::from_secs(3),
        interval: Duration::from_secs(1),
        required_only: true,
        allow_no_checks: false,
    };
    super::super::compute(&runner, &clock, &global, &ctx, &args).unwrap()
}

#[test]
fn empty_history_is_not_registered_when_a_read_times_out_before_at_or_after_budget() {
    for completed_polls in [0, 1, 2] {
        for elapsed_ms in [2900, 3000, 3100] {
            let outcome =
                interrupted_wait(completed_polls, false, Duration::from_millis(elapsed_ms));
            let WaitOutcome::NotRegistered(snapshot, NotRegisteredCause::BudgetExpired) = outcome
            else {
                panic!("expected NotRegistered: polls={completed_polls}, elapsed={elapsed_ms}");
            };
            assert!(snapshot.checks.is_empty());
            assert_eq!(snapshot.duration_ms, Some(elapsed_ms));
            assert_eq!(
                snapshot.state,
                if completed_polls == 0 {
                    "pending"
                } else {
                    "success"
                }
            );
        }
    }
}

#[test]
fn observed_pending_check_still_times_out_when_a_read_is_interrupted() {
    for elapsed_ms in [2900, 3000, 3100] {
        let outcome = interrupted_wait(1, true, Duration::from_millis(elapsed_ms));
        let WaitOutcome::TimedOut(snapshot) = outcome else {
            panic!("expected TimedOut with an observed pending check");
        };
        assert_eq!(snapshot.checks.len(), 1);
        assert_eq!(snapshot.checks[0].state, "pending");
        assert_eq!(snapshot.duration_ms, Some(elapsed_ms));
    }
}
