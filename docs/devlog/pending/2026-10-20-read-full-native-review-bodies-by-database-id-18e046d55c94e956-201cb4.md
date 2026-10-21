## 2026-10-20 - Read full native review bodies by database ID
<!-- devlog-id: 2026-10-20-read-full-native-review-bodies-by-database-id-18e046d55c94e956-201cb4 -->

### Result

- Added an opt-in complete native review read with bound provenance, explicit
  completeness and the existing body limit.

### Why / context

- Bounded summaries can hide review qualifiers; reuse the verified native read
  while preserving summary output and handoff admission behavior.

### Evidence

- Captured the missing-option failure before production edits; nine full-read,
  nine summary and 74 handoff regressions plus scoped clippy passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2465>
