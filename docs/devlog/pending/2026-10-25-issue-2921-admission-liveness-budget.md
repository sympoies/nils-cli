## 2026-10-25 - Bound active-session admission liveness
<!-- devlog-id: 2026-10-25-issue-2921-admission-liveness-budget -->

### Result

- Admission occupancy now bounds aggregate same-boot liveness probes below the
  coordination lock wait. Skipped or timed-out probes remain counted as active,
  preserving conservative capacity accounting.

### Why / context

- Sequential retained-session liveness calls could hold the admission lock
  longer than competing creators wait, causing retryable timeouts while
  capacity remained available.

### Evidence

- The new concurrent-create integration regression reproduced
  coordination-lock-timeout before the fix and passed after it; cargo fmt
  --package nils-agent-session -- --check passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2921>
