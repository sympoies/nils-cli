## 2026-11-20 - Stabilize agent-session fixtures in the environment-isolation lane
<!-- devlog-id: 2026-11-20-issue-3824-env-isolation -->

### Result

- Lifecycle lock exhaustion uses a fixture clock and holds real reader locks
  through the busy response. The quarantined restart fixture signals when its
  pane starts and exits.
- The poisoned-environment guard fails when nextest reports a failed test run
  and keeps test output out of the gate log.

### Why / context

- Timed reader release and subprocess scheduling could change the result of the
  two library tests under load.

### Evidence

- The unchanged library lane passed both targets. In 64 concurrent one-CPU
  nextest copies per target, lifecycle exhaustion failed 64 times and the
  quarantined restart failed 15 times. Both passed all 64 copies after the
  fixture changes. A synthetic nextest exit 100 now fails the guard.

### Links

- <https://github.com/sympoies/nils-cli/issues/3824>
