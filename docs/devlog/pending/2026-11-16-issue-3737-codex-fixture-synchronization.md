## 2026-11-16 - Synchronize Codex contention and projection fixtures
<!-- devlog-id: 2026-11-16-issue-3737-codex-fixture-synchronization -->

### Result

- Control real lock-attempt boundaries in unit-only cancellation fixtures and
  await the scheduled projection fail-close task before checking persisted
  state.

### Why / context

- Independent lock-release sleeps could forward a request intended to exhaust,
  while a short projection polling window raced its background write. Exact
  response and state assertions remain, including the contention fixture
  eight-second bound; shipped retry and lock budgets are unchanged.

### Evidence

- With 32 copies pinned to one CPU at nice priority 19, contention improved
  from 23/32 to 32/32 using eight competing CPU workers; projection improved
  from 10/32 to 32/32 using 32 competing workers. Both fixed cases also passed
  32/32 with eight workers.

### Links

- <https://github.com/sympoies/nils-cli/issues/3737>
