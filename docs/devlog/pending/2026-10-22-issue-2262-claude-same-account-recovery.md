## 2026-10-22 - Quarantine Claude auth failures and resume after newer publication
<!-- devlog-id: 2026-10-22-issue-2262-claude-same-account-recovery -->

### Result

- Added token-free launch checkpoints, bounded same-account recovery, and
  fenced unknown-effect receipts without prompt replay.

### Why / context

- Materialization and timestamps cannot identify failed credential generations.
  Recovery preserves conversation identity and exact stopped-runtime proofs
  while treating untracked effects as unknown.

### Evidence

- Two prompt-quarantine regressions failed before production edits. Nine
  focused recovery tests, publication/account tests, incident tests and scoped
  Clippy pass; unsafe stop boundaries remain refused.

### Links

- <https://github.com/sympoies/nils-cli/issues/2262>
