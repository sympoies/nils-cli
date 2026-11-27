## 2026-11-27 - Bound slow agent-session CI acceptance tests
<!-- devlog-id: 2026-11-27-issue-4155-agent-session-test-bounds -->

### Result

- Federation delivery polls retry the typed busy-store result within their own
  deadlines. CI caps federation at 150 seconds and auth recovery at 90 seconds
  without a nextest retry.

### Why / context

- A release merge-group runner exhausted its job budget during workspace tests
  after a slow setup and build. Both named tests had finished, but federation
  had retried a read-only store contention failure and could previously have
  consumed 20 minutes if it hung.

### Evidence

- Compared both fixtures on main and v1.44.0 under the Linux CI nextest
  profile; reproduced the post-restart failure by holding the fixture
  federation journal lock.

### Links

- <https://github.com/sympoies/nils-cli/issues/4155>
