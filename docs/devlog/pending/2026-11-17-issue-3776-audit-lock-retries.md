## 2026-11-17 - Cover bounded identity audit lock retries
<!-- devlog-id: 2026-11-17-issue-3776-audit-lock-retries -->

### Result

- Added deterministic coverage of retry success, timeout, and interrupted
  attempts.

### Why / context

- A zero-budget contention test did not exercise retry waits.

### Evidence

- A sleep-removal mutant survived the existing contention test and failed the
  new test; focused identity tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3776>
