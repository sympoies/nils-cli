## 2026-11-17 - Account brokers distinguish no-account from process faults
<!-- devlog-id: 2026-11-17-issue-3577-typed-account-broker-result -->

### Result

- Both broker protocols reserve exit 10 for no-account; other process exits
  report broker unavailability, and successful null or missing selections are
  invalid responses.

### Why / context

- A bare process failure cannot prove account absence. The reserved status
  works even when a fault cannot emit JSON, and brokers-first rollout lets
  existing clients consume the updated broker before the stricter client is
  deployed.

### Evidence

- Retained pre-edit nextest run: four contract regressions failed for
  readiness, request-time handling, untyped failover faults and empty success
  selections; the exit-10 characterization passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3577>
