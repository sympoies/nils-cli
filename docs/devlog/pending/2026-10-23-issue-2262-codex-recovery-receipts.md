## 2026-10-23 - Fence Codex same-account authentication refresh operations
<!-- devlog-id: 2026-10-23-issue-2262-codex-recovery-receipts -->

### Result

- Persist token-free runtime-fenced refresh receipts, refuse duplicate and
  uncertain replay, and bound consecutive refresh requests and response writes.

### Why / context

- Official token refresh already existed, but replay identities and retry
  budgets did not survive controller replacement.

### Evidence

- Three meaningful pre-fix regressions failed; focused broker, WebSocket,
  binding, crash and deadline validation passes.

### Links

- <https://github.com/sympoies/nils-cli/issues/2262>

### Follow-ups

- Policy-gated failover and real-provider acceptance remain separately owned.
