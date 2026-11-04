## 2026-11-05 - Retire plain-text session notices
<!-- devlog-id: 2026-11-05-issue-3333-retire-plain-notices -->

### Result

- Legacy stored-title, mailbox and continuation text no longer receives the
  notice row class and renders as ordinary provider input. Mailbox writers use
  current envelopes; disabled writers and unsupported notice routes send
  nothing. The obsolete terminal fallback and its composer recovery path are
  removed. Codex context-frame recognition remains.

### Why / context

- The maintainer retired the plain formats so producers and consumers share one
  current notice contract. Consumers must repin the history-row fixture and
  update their installed daemon to adopt the reclassification. Unread mail
  remains available through explicit inbox reads.

### Evidence

- Four focused regression checks failed for the intended classification,
  producer and gate-off gaps before production edits; final scoped validation
  is recorded in the linked PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3333>
