## 2026-10-20 - Retained RED and GREEN evidence can be bound after initialization was missed
<!-- devlog-id: 2026-10-20-issue-2631-retained-test-evidence -->

### Result

- Added explicit retro-bound v2 provenance, retained log digests,
  original-baseline binding, and aggregate verify recovery steps.

### Why / context

- Retained pre-edit evidence should remain auditable without treating a late
  import as contemporaneous capture or bypassing remaining delivery
  requirements.

### Evidence

- Captured failing regression tests before implementation; the focused
  test-first-evidence integration suite passed 27 tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/2631>
