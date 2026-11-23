## 2026-11-24 - Retry unacquired hosted macOS CI jobs
<!-- devlog-id: 2026-11-24-ci-retry-hosted-macos-3982 -->

### Result

- Merge-group and nightly CI retry only required macOS jobs that the provider
  failed to acquire, once, on the same commit.

### Why / context

- The required check must remain pending while capacity recovers. A rerun after
  the original failure can arrive after the queue entry is removed.

### Evidence

- Recorded a red retry-decision fixture, then passed the capacity,
  full-validation, delivery workflow and release workflow fixture suites.

### Links

- <https://github.com/sympoies/nils-cli/issues/3982>
