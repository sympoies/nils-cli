## 2026-10-21 - Clear unconfirmed native mailbox wake text from the composer
<!-- devlog-id: 2026-10-21-issue-2536-clear-unconfirmed-wake -->

### Result

- A native mailbox wake whose submission cannot be confirmed no longer leaves
  the pasted notification in an idle Claude or Codex composer; the exact pasted
  text is cleared and the wake is retried, and residual text is surfaced as a
  typed Unknown state only when clearing cannot be proven safe.

### Why / context

- A pasted but unsubmitted notification kept the session idle until unrelated
  input pressed Enter and concatenated the stale text with it.

### Evidence

- Fake-tmux regressions failed on the unfixed wake path (stale composer text
  after swallowed or unconfirmed Enter) and pass with the fix for both
  providers; operator drafts and mixed composer text are never submitted or
  deleted.

### Links

- <https://github.com/sympoies/nils-cli/issues/2536>
