## 2026-11-16 - Deterministic native composer transaction fixtures under contention
<!-- devlog-id: 2026-11-16-issue-3733-native-composer-fixtures -->

### Result

- Run the unchanged nine-case native composer module through a shared synthetic
  transaction fixture with scoped unit-only hold/capture budgets and the
  existing retry injection.

### Why / context

- Python captures raced the production one-second cap under contention.
  Paste-settle also narrowed captures to its short remaining interval, so the
  fixture budget is applied at the styled subprocess boundary. All success,
  exact refusal, draft preservation and idempotency assertions remain
  unchanged; shipped budgets are unchanged.

### Evidence

- Baseline module passed 0/32 concurrent copies on one CPU; final module passed
  32/32 (288 test executions), plus 9/9 under normal load and
  workspace-selected JSON comparisons on Rust 1.100.0.

### Links

- <https://github.com/sympoies/nils-cli/issues/3733>
