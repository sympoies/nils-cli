## 2026-11-16 - Isolate seeded catalog scan fixture
<!-- devlog-id: 2026-11-16-isolate-seeded-catalog-scan-fixture-18e852571761c5b4-4b9bb -->

### Result

- The seeded catalog test keeps generation-stamp ancestors inside its fixture
  and uses a frozen test clock.

### Why / context

- A crowded temporary root exceeded the per-directory entry limit; scheduler
  delays could also expire the production scan deadline.

### Evidence

- The focused test failed with 10002 synthetic temporary-root entries before
  the fix; the catalog scan tests passed after isolation.

### Links

- <https://github.com/sympoies/nils-cli/issues/3726>
