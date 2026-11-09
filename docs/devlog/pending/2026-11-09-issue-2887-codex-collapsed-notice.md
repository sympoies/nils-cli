## 2026-11-09 - Recognize owned collapsed Codex notice elements
<!-- devlog-id: 2026-11-09-issue-2887-codex-collapsed-notice -->

### Result

- Long guarded Codex notices and terminal mailbox wakes recognize a matching
  single collapsed paste element, prove its atomic cursor path and submit once;
  safe refusal cleanup deletes one editing unit.

### Why / context

- Codex 0.162.1 labels large pastes by their Unicode scalar count after newline
  normalization. Counting bytes or deleting the expanded character count is
  incorrect for this representation; a literal lookalike must still fail cursor
  proof.

### Evidence

- The native 1153-character placeholder fixture failed at submit with
  pasted=true, submitted=false, cleared=false and remaining_input=unobservable
  before production edits. Three focused collapsed-notice cases passed after
  the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/2887>
