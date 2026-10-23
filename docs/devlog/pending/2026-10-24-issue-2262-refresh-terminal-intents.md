## 2026-10-24 - Preserve Codex refresh after terminal account intents
<!-- devlog-id: 2026-10-24-issue-2262-refresh-terminal-intents -->

### Result

- Official same-account refresh refuses only queued or applying account
  intents; failed and malformed intents retain their evidence and prompt fences
  while allowing current-account credential recovery.

### Why / context

- A historical next-account refusal must not strand an otherwise current bound
  runtime after authentication loss.
- The rollback compatibility follow-up is already covered by the strict-store
  rollback procedure in the auth failover spec; no reader relaxation is needed.

### Evidence

- Failed and Invalid intent regressions failed with account-switch-pending
  before the fix and passed afterward; queued and applying refusal coverage
  also passed with TMPDIR=/tmp.

### Links

- <https://github.com/sympoies/nils-cli/issues/2262>
