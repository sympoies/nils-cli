## 2026-11-18 - Retry mailbox polling contention within the original deadline
<!-- devlog-id: 2026-11-18-issue-2656-mailbox-polling-contention -->

### Result

- Revision and any-unread wait polling now recover transient registry lock
  timeouts using bounded sleeping retries within the original monotonic
  deadline, retaining cancellation and mode-specific timeout contracts.

### Why / context

- A notification could be consumed before a contended registry read failed, so
  waiting for another notification could lose the arrival. Direct acquisition
  retries preserve the arrival without adding idle notification wakeups.
  Authentication-time lock contention remains outside this change.

### Evidence

- Before release-path edits, all twenty synchronized waiters returned
  coordination-lock-timeout and both deadline regressions failed. The
  controlled real-lock storm passes 20 of 20 runs after the fix, observing all
  400 eligible waiter arrivals; cancellation, permanent-error classification
  and event-filter unit cases pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2656>
