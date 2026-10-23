## 2026-10-24 - Keep dispatch wall latency as a non-required nightly signal
<!-- devlog-id: 2026-10-24-issue-1705-dispatch-performance-signal -->

### Result

- Move only the dispatch wall-latency benchmark out of default required
  testing; run its unchanged p95 limits without retries in the existing nightly
  Linux context and retain separate JUnit evidence.

### Why / context

- Loaded repetitions and per-child scheduler accounting reproduced wall-budget
  failures while dispatch CPU work stayed below the limits. Nextest pool
  exclusivity does not isolate the OS scheduler.

### Evidence

- Baseline: 7 passed, 1 failed, 2 unrun; diagnostic: 9 passed, 1 failed with
  recorded runqueue delay. Identity-churn: 10 of 10 passed under an explicit
  macOS cap waiver; remains unconfirmed.

### Links

- <https://github.com/sympoies/nils-cli/issues/1705>
