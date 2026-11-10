## 2026-11-10 - Honor the mailbox retry deadline in Claude wake fixtures
<!-- devlog-id: 2026-11-10-issue-3514-wake-retry-deadline -->

### Result

- The clear and stale-stash fixtures wait for the notification receipt retry
  deadline and verify discovery eligibility before authorizing the retry.

### Why / context

- Immediate direct redispatch bypassed discovery backoff and depended on guard
  work crossing the next clock second. Loaded due-candidate assertions failed
  while receipts stayed queued and unclaimed; no product change was needed.

### Evidence

- Both diagnostic precondition assertions failed under nextest with 8 test
  threads and 32 CPU contenders through the repository resource gate.

### Links

- <https://github.com/sympoies/nils-cli/issues/3514>
