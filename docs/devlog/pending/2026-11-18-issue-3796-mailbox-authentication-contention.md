## 2026-11-18 - Include mailbox authentication in the wait deadline
<!-- devlog-id: 2026-11-18-issue-3796-mailbox-authentication-contention -->

### Result

- Revision and any-unread waits share one budget across authentication and
  polling, and retry only registry lock timeouts.

### Why / context

- Both wait modes previously took a fixed two-second registry lock before their
  wait clocks started. Reusing the polling acquisition keeps capability checks
  and delegation audit on the existing path.

### Evidence

- Both held-lock wait modes returned coordination-lock-timeout on the recorded
  pre-fix run. All 12 unchanged any-unread integration tests, including idle
  CPU, pass with the shared budget.

### Links

- <https://github.com/sympoies/nils-cli/issues/3796>
