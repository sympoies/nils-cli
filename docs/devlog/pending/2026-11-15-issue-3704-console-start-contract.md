## 2026-11-15 - Clarify Console start operation IDs, title mode, and launch origin
<!-- devlog-id: 2026-11-15-issue-3704-console-start-contract -->

### Result

- Invalid Console operation IDs now name the canonical UUIDv7 requirement.
  Start help and the Console spec explain automatic title replacement and
  retained launch origin for no-parent lineage roots.

### Why / context

- Acceptance found a generic request-shape refusal that described the submitted
  combination as allowed, and help obscured the existing title and origin
  contracts. Valid UUIDv7 submissions already reconcile; no wire or lineage
  behavior changed.

### Evidence

- Before production edits, three diagnostic/help regressions failed while the
  valid CLI replay test passed. After the fix, all 14 focused Console start
  tests and the existing synthetic Console resume suite passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3704>
