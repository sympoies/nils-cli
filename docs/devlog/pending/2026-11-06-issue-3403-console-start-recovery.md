## 2026-11-06 - Recover known service console creations after a lost start answer
<!-- devlog-id: 2026-11-06-issue-3403-console-start-recovery -->

### Result

- Preserve a known created-session ID on unavailable upstream start answers and
  recover it through the existing operation receipt.

### Why / context

- Discarding creation evidence made read-only reconciliation ambiguous even
  when the upstream service knew the child identity. No second create or
  cleanup is needed.

### Evidence

- Verified RED on current main: reconciliation returned ambiguous with no child
  after an unavailable answer carrying the created ID.

- Resource-gated service console start suite: 13 passed, including recovery after restart, definitive not-created, and genuinely unknown results.

### Links

- <https://github.com/sympoies/nils-cli/issues/3403>
