## 2026-10-24 - Authorize provenance upstream reads in their repository scope
<!-- devlog-id: 2026-10-24-issue-2871-upstream-read-scope -->

### Result

- Pinned upstream commit and tree reads use the upstream repository api_read
  identity scope; backend refusals retain their typed causes and recovery
  metadata.
- Exact added paths under mapped directories remain visible as additions,
  retain ordinary inspection limits, and require independent confirmation.
  Omitted text patches are reconstructed from verified immutable blobs at
  the three-dot merge base and head, with that base bound into confirmation;
  missing binary patches identify the exact unavailable evidence.

### Why / context

- A PR repository identity scope cannot authorize a different upstream owner.
  Narrow upstream reads preserve fail-closed policy selection and restore the
  PR scope afterward. Mapped binary assets can remain verified while adjacent
  new notices retain ordinary inspection and comparison limits.

### Evidence

- Regression-first CLI tests reproduced both generic provenance refusals.
  Focused test-integrity tests pass with read-only upstream authorization,
  denied repository/operation cases, restored downstream scope, declared
  notice admission, omitted large text patches, binary refusals, blob
  corruption checks, modified/removed/renamed paths after a moved base tip,
  and unchanged residual bounds.

### Links

- <https://github.com/sympoies/nils-cli/issues/2871>
