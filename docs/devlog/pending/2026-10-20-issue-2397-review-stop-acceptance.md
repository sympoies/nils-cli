## 2026-10-20 - Governed terminal risk acceptance after review hard stops
<!-- devlog-id: 2026-10-20-issue-2397-review-stop-acceptance -->

### Result

- Review-loop observations can append a maintainer-approved terminal acceptance
  receipt, preserving the stopped inventory, original stop, named fingerprints,
  approval reference, and repair budgets.

### Why / context

- An extension authorizes another identical repair attempt; accepting remaining
  risk needs a separate inventory-bound approval so convergence can complete
  without extending or resetting the repair budget.

### Evidence

- Captured the approved acceptance preflight regression before implementation,
  then passed 46 focused review-loop integration tests covering acceptance,
  authorization failures, immutable history, convergence, and existing recovery
  paths.

### Links

- <https://github.com/sympoies/nils-cli/issues/2397>
