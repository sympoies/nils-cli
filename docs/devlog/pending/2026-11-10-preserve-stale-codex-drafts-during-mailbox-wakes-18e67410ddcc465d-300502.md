## 2026-11-10 - Preserve stale Codex drafts during mailbox wakes
<!-- devlog-id: 2026-11-10-preserve-stale-codex-drafts-during-mailbox-wakes-18e67410ddcc465d-300502 -->

### Result

- Mailbox wake cleanup uses exact-draft cursor proof for saved owner text.

### Why / context

- Notice paste proof cannot prove arbitrary owner drafts and left an eligible
  wake queued after the draft was saved.

### Evidence

- A focused terminal fixture failed before the fix and passed after it,
  checking byte-exact saved text and one submission.

### Links

- <https://github.com/sympoies/nils-cli/issues/3484>
