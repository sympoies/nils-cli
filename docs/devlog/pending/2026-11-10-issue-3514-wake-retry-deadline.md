## 2026-11-10 - Honor the mailbox retry deadline in wake fixtures
<!-- devlog-id: 2026-11-10-issue-3514-wake-retry-deadline -->

### Result

- The Claude clear/stale-stash and Codex stale-draft fixtures wait for the
  notification receipt retry deadline and verify discovery eligibility before
  redispatch.

### Why / context

- Immediate direct redispatch bypassed discovery backoff and depended on guard
  work crossing the next clock second. Loaded due-candidate assertions failed
  while receipts stayed queued and unclaimed. Codex CI receipts showed both
  deferrals in one clock second, before the next retry was due; no product
  change was needed.

### Evidence

- Both diagnostic precondition assertions failed under nextest with 8 test
  threads and 32 CPU contenders through the repository resource gate. The same
  Codex precondition then failed under load before its fixture fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3514>
