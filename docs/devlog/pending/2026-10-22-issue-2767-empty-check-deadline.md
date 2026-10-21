## 2026-10-22 - Make empty check-history deadline outcomes deterministic
<!-- devlog-id: 2026-10-22-issue-2767-empty-check-deadline -->

### Result

- Polling expiry now reports checks_not_registered whenever no check has been
  observed, including interrupted backend reads; observed unfinished checks
  still report checks_timeout.

### Why / context

- The macOS release gate exposed scheduling-dependent classification between
  poll-sleep expiry and an interrupted snapshot read.

### Evidence

- Injected-clock regression failed before the fix; all 19 wait-checks unit
  tests pass after the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/2767>
- <https://github.com/sympoies/nils-cli/issues/2741>
