## 2026-11-15 - Restore guarded composer diagnostics with deterministic coverage
<!-- devlog-id: 2026-11-15-issue-3725-composer-diagnostics-reland -->

### Result

- Restore observation diagnostics with test-only retry budget injection.

### Why / context

- The preceding diagnostics were reverted after a timing-sensitive fixture
  blocked the merge queue.

### Evidence

- Baseline stress failed under contention; the deterministic fixture passed 32
  concurrent runs on one CPU.

### Links

- <https://github.com/sympoies/nils-cli/issues/3725>
