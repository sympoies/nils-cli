//! A single budget follows every backend invocation in the polling operation.
use super::*;
use crate::backend::{BackendOutput, BackendSuccess};
struct DeadlineRunner<'a, R, C> {
    inner: &'a R,
    clock: &'a C,
    deadline: Instant,
}

impl<R: BackendRunner, C: Clock> DeadlineRunner<'_, R, C> {
    fn remaining(&self) -> Result<Duration, ForgeError> {
        let remaining = self.deadline.saturating_duration_since(self.clock.now());
        if remaining.is_zero() {
            Err(ForgeError::unavailable(
                pr_checks::schema_err(),
                "backend_timeout",
                "checks operation exhausted its timeout budget",
                None,
            ))
        } else {
            Ok(remaining)
        }
    }
}

impl<R: BackendRunner, C: Clock> BackendRunner for DeadlineRunner<'_, R, C> {
    fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
        self.run_with_timeout(call, None)
    }
    fn run_with_timeout(
        &self,
        call: &BackendCall,
        timeout: Option<Duration>,
    ) -> Result<BackendSuccess, ForgeError> {
        let remaining = self.remaining()?;
        let output = self
            .inner
            .run_with_timeout(call, Some(timeout.map_or(remaining, |t| t.min(remaining))))?;
        self.remaining()?;
        Ok(output)
    }
    fn run_raw(&self, call: &BackendCall) -> Result<BackendOutput, ForgeError> {
        self.run_raw_with_timeout(call, None)
    }
    fn run_raw_with_timeout(
        &self,
        call: &BackendCall,
        timeout: Option<Duration>,
    ) -> Result<BackendOutput, ForgeError> {
        let remaining = self.remaining()?;
        let output = self
            .inner
            .run_raw_with_timeout(call, Some(timeout.map_or(remaining, |t| t.min(remaining))))?;
        self.remaining()?;
        Ok(output)
    }
}

pub(super) fn poll_until_terminal_or_timeout<R: BackendRunner, C: Clock>(
    runner: &R,
    clock: &C,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    args: &PrWaitChecksArgs,
    snapshot_args: &PrChecksArgs,
) -> Result<WaitOutcome, ForgeError> {
    let start = discovery::start().unwrap_or_else(|| clock.now());
    let deadline = discovery::deadline().unwrap_or(start + args.timeout);
    let runner = DeadlineRunner {
        inner: runner,
        clock,
        deadline,
    };
    let presence = CheckPresence::from_allow_no_checks(args.allow_no_checks);
    let mut ever_saw_a_check = false;
    let mut observed_snapshot = false;
    let mut workflows_probed = false;
    // Incomplete snapshots cannot replace the last fully observed state.
    let mut last = pr_checks::aggregate(ctx, Vec::new(), args.required_only, None);
    last.state = "pending";
    loop {
        let result = pr_checks::snapshot_for_gate(&runner, global, ctx, snapshot_args);
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) if error.kind() == "backend_timeout" || clock.now() >= deadline => {
                // An interrupted read proves neither an empty current head nor
                // absence of registration, even after earlier empty snapshots.
                return Ok(WaitOutcome::TimedOut(with_duration(
                    last,
                    ms_between(start, clock.now()),
                )));
            }
            Err(error) => return Err(error),
        };
        // Backends in tests may not implement timeouts; terminal results still
        // need the same deadline check before they can be admitted.
        let snapshot = gate_visible_checks_when_none_required(ctx, snapshot_args, snapshot);
        let admitted_at = clock.now();
        if admitted_at >= deadline {
            return Ok(expired(
                last,
                start,
                admitted_at,
                observed_snapshot,
                ever_saw_a_check,
            ));
        }
        observed_snapshot = true;
        last = with_duration(snapshot, ms_between(start, admitted_at));
        ever_saw_a_check |= !crate::ops::required_check_gate::nothing_was_checked(&last);
        if is_terminal(&last, presence) {
            return Ok(if last.state == "success" {
                WaitOutcome::Success(last)
            } else {
                WaitOutcome::Failed(last)
            });
        }
        let now = clock.now();
        if !ever_saw_a_check
            && !workflows_probed
            && presence == CheckPresence::Required
            && ctx.provider == Provider::GitHub
            && now.saturating_duration_since(start) >= NO_CI_GRACE_PERIOD
        {
            workflows_probed = true;
            let no_workflows = github_repo_has_no_workflows(&runner, ctx);
            if clock.now() >= deadline {
                return Ok(expired(
                    last,
                    start,
                    clock.now(),
                    observed_snapshot,
                    ever_saw_a_check,
                ));
            }
            if no_workflows {
                return Ok(WaitOutcome::NotRegistered(
                    with_duration(last, ms_between(start, clock.now())),
                    NotRegisteredCause::NoWorkflows,
                ));
            }
        }
        let remaining = deadline.saturating_duration_since(clock.now());
        clock.sleep(args.interval.min(remaining));
        if clock.now() >= deadline {
            return Ok(expired(
                last,
                start,
                clock.now(),
                observed_snapshot,
                ever_saw_a_check,
            ));
        }
    }
}

fn expired(
    snapshot: PrChecksPayload,
    start: Instant,
    now: Instant,
    observed_snapshot: bool,
    ever_saw_a_check: bool,
) -> WaitOutcome {
    let snapshot = with_duration(snapshot, ms_between(start, now));
    if observed_snapshot && !ever_saw_a_check {
        WaitOutcome::NotRegistered(snapshot, NotRegisteredCause::BudgetExpired)
    } else {
        WaitOutcome::TimedOut(snapshot)
    }
}
