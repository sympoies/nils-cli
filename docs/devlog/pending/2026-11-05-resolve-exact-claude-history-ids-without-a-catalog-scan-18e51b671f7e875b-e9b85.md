## 2026-11-05 - Resolve exact Claude history IDs without a catalog scan
<!-- devlog-id: 2026-11-05-resolve-exact-claude-history-ids-without-a-catalog-scan-18e51b671f7e875b-e9b85 -->

### Result

- Exact Claude history searches probe project transcript names directly and
  retain the matching conversation for retrieval.

### Why / context

- A bounded catalog scan could truncate before reaching an active transcript on
  a busy host.

### Evidence

- Synthetic catalog regression exhausts the scan entry limit and resolves the
  matching transcript.

### Links

- <https://github.com/sympoies/nils-cli/issues/3358>
