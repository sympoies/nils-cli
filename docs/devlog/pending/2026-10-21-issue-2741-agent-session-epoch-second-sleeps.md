## 2026-10-21 - Remove epoch-second sleeps from agent-session tests
<!-- devlog-id: 2026-10-21-issue-2741-agent-session-epoch-second-sleeps -->

### Result

- The three Claude completion tests freeze an injectable activity clock and
  advance it past the one-second quiet window instead of sleeping; one
  real-clock test keeps the production path covered by waiting on the settled
  phase with a 30 s bound.
- The message-expiry and work-context claim-expiry integration tests write an
  already-expired epoch into the registry instead of sleeping across an
  epoch-second boundary.
- The retitle fallback queue test runs on paused tokio time; the restart
  record-lock test holds the lock until the restart returns; the broker
  readiness test holds the registry lock until the sidecar's next heartbeat;
  the usage refresh tests hold the stub refresh on a gate file instead of a
  fixed 1 s delay.

### Why / context

- Part of making full validation load tolerant so test threads can be raised
  (#2741, batch B5).

### Evidence

- Eleven target tests: summed test time about 31 s before and about 19 s after
  on an idle host; 10 of 10 runs green with four nice-19 busy processes and the
  run pinned to the same two cores. The unchanged tests also passed under that
  load, so the gain is speed and removal of wall-clock assumptions, not a
  reproduced failure.
- Mutation check: widening the production quiet window from 1 s to 2 s fails
  the clock-based completion test.

### Links

- <https://github.com/sympoies/nils-cli/issues/2741>

### Follow-ups

- The one-second usage cache interval and the lock-wait tests in
  contention_tests and codex_app_server keep real-time waits; they are
  real-time behavior, not epoch-second sleeps.
