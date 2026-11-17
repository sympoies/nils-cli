## 2026-11-17 - Distinguish continuation broker faults from empty account selection
<!-- devlog-id: 2026-11-17-issue-3775-continuation-broker-faults -->

### Result

- Continuation failover retains typed broker causes in the existing auto-resume
  attention projection without per-poll warnings or schedule changes.

### Why / context

- Discarding account-selection errors made broker faults indistinguishable from
  an empty candidate set. The existing failure_reason field provides one
  current diagnostic without creating a second alert channel.

### Evidence

- Test-first regression reproduced no_account_available for malformed broker
  output while exit-10 no-account and successful selection guards passed.
- Fresh baseline red and focused green cover repeated faults, recovery to
  no-account, unchanged scheduling/retry counters, and per-tick public/durable
  state privacy; the focused run emitted no warnings or broker output.

### Links

- <https://github.com/sympoies/nils-cli/issues/3775>
