## 2026-10-21 - Make agent-session process-group timeout fixtures load tolerant
<!-- devlog-id: 2026-10-21-issue-2741-agent-session-timeout-fixtures -->

### Result

- Process-group timeout fixtures give a child that must publish a pid a budget
  well above interpreter startup, and bound "returned promptly" checks far
  below the sleep a surviving group would hold; post-kill reap polls wait up to
  10 s and return as soon as the group is gone.
- The serve "let it start" sleeps wait on conditions instead: a finished scan
  or parked waiters in the provider-discovery registry, and a parked flock
  waiter listed in /proc/locks for the record-lock fixtures. Sleeps whose
  outcome never depended on them are removed.
- Restart readiness probes stall for 20 s against a 2 s budget and an 8 s
  ceiling, so an uncapped probe still fails; the usage-helper fixture now fails
  when a surviving descendant keeps its pipes open.
- The delete cgroup fixture no longer depends on the caller's locale: tmux
  prints tabs as underscores outside a UTF-8 locale, so the pane anchor runs
  with an explicit one.

### Why / context

- Part of making full validation load tolerant so test threads can be raised
  (#2741, batch B4).

### Evidence

- Pinned to one CPU with four nice-19 busy loops, the restart stalled-status
  test failed on 2 of 4 baseline runs (waited 2.5 s past a 1 s timeout); the
  changed set passed 4 of 4 runs, 15 tests each, with a scrubbed environment
  and no LANG.
- Mutating the product group kill (readiness, usage helper, capability probe,
  account broker) or the readiness probe cap made the matching fixture fail.

### Links

- <https://github.com/sympoies/nils-cli/issues/2741>

### Follow-ups

- The pid-publication races (100 ms and 250 ms budgets) and the delete
  identity-churn fixture did not fail on the build host before the change, so
  they have no RED of their own.
