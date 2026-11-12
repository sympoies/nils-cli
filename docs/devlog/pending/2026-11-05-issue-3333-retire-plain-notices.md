## 2026-11-05 - Retire plain-text session notices
<!-- devlog-id: 2026-11-05-issue-3333-retire-plain-notices -->

### Result

- Legacy stored-title, mailbox and continuation text no longer receives the
  notice row class and renders as ordinary provider input. Mailbox writers use
  current envelopes; disabled writers and unsupported notice routes send
  nothing. The obsolete terminal fallback and its composer recovery path are
  removed. Codex context-frame recognition remains.

- Hook-only wakes remain supported. The hook accepts current mailbox wake
  envelopes and forwards the explicit writer gate to its companion; retired
  plain replies are rejected.

### Why / context

- The maintainer retired the plain formats so producers and consumers share one
  current notice contract. Consumers must repin the history-row fixture and
  update their installed daemon to adopt the reclassification. Unread mail
  remains available through explicit inbox reads.

- Deploy matching agent-session and agent-hook binaries together. Consumers
  must also repin the shared history-row fixture with this release.

### Evidence

- Four focused regression checks failed for the intended classification,
  producer and gate-off gaps before production edits; final scoped validation
  is recorded in the linked PR. Hook boundary tests also failed before the
  envelope reader and writer-gate forwarding changed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3333>
