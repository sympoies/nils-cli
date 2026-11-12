## 2026-11-09 - Explain unreadable multi-row Claude drafts
<!-- devlog-id: 2026-11-09-issue-3454-wrapped-draft-refusal -->

### Result

- Mailbox wakes retain a precise multi-row composer refusal, and direct replies
  explain how to proceed.

### Why / context

- Wrapped rows do not expose exact separator bytes, so draft preservation must
  not guess.

### Evidence

- Focused draft guard tests and baseline regression fixture.

### Links

- <https://github.com/sympoies/nils-cli/issues/3454>
