## 2026-11-18 - Stabilize the mailbox capacity and auth quarantine fixtures
<!-- devlog-id: 2026-11-18-issue-3809-agent-session-fixtures -->

### Result

- The full-mailbox test uses a scoped fixture epoch and the replacement
  quarantine tests consume committed fixture publication data at the broker
  boundary.

### Why / context

- Under one-CPU contention, the mailbox fixture expired during acknowledgements
  and auth recovery sometimes stopped before readiness after relaunch.

### Evidence

- Before the changes, nextest passed all 85 selected tests. With 64 workers on
  one CPU, mailbox acknowledgement failed in 64 of 64 copies and auth recovery
  missed the readiness event in 7 of 64 copies.

### Links

- <https://github.com/sympoies/nils-cli/issues/3809>
