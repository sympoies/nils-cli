## 2026-10-21 - Make git-cli dirty-checkout adoption tests load tolerant
<!-- devlog-id: 2026-10-21-issue-2741-git-cli-load-tolerant-deadlines -->

### Result

- The dirty-checkout adoption process-supervisor and cleanup tests wait up to
  20 s for conditions that end as soon as they hold (fixture PID publication,
  process exit, output budgets) instead of 0.5-2 s windows.
- Tests whose subject is a deadline keep a real-time check but with wide bands:
  the budget is far below the stall, which is far below the ceiling (absolute
  supervisor deadline, one absolute challenge-descriptor read deadline, and the
  deadline-crossing cleanup, which now holds the stopped process until the
  deadline instead of a fixed 75 ms).
- The detached double-fork supervisor test now signals the supervisor process,
  as production cleanup does, instead of its whole process group, which also
  killed the supervised target and raced the supervisor status check.
- The descendant scan treats a task directory that vanishes mid-scan (ESRCH) as
  a departed process rather than a resource failure, like the other /proc
  reads.

### Why / context

- Part of making full validation load tolerant so test threads can be raised
  (#2741, batch B8).

### Evidence

- Baseline on origin/main with the suite pinned to 2 CPUs and oversubscribed:
  26 of 60 iterations of the supervisor lifecycle and status tests failed
  (detached_double_fork 50 failures, supervisor_ownership_eof 2). The same run
  after the change: 60 of 60 passed.
- Snapshot CLI tests under the same pressure: 3 of 100 iterations failed with a
  process-cleanup resource error from the task directory read before the ESRCH
  fix; 0 of 150 after it.
- Full nils-git-cli suite, 3 runs under 4 nice-19 busy processes: all 482 tests
  green each run; touched-crate clippy clean.

### Links

- <https://github.com/sympoies/nils-cli/issues/2741>

### Follow-ups

- Rare residual failures (about 1 in 100-150 iterations under the same 2-CPU
  pressure) remain in the snapshot worker-executable validation ("changed
  during validation" / "could not be resolved") and are not addressed here.
- concurrent_adoption_consumes_a_challenge_exactly_once keeps its fixed 100 ms
  contention window and its nextest full-pool reservation; the reservation
  stays with the nextest group batch.
