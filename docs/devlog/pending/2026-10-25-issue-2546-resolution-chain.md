## 2026-10-25 - Preserve chained final-validation resolutions
<!-- devlog-id: 2026-10-25-issue-2546-resolution-chain -->

### Result

- Final evidence verification follows investigated resolution chains to a
  current passing command without rewriting historical attempts.

### Why / context

- A valid earlier resolution could become blocked when its passing command
  later failed but was itself resolved by another canonical pass.

### Evidence

- Captured the CLI regression before production edits; nine focused verifier
  tests and the source-bound forge draft-preview regression passed. Broken
  references, unresolved endpoints and self or multi-command cycles remain
  rejected.

### Links

- <https://github.com/sympoies/nils-cli/issues/2546>
