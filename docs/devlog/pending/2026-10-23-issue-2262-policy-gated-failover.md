## 2026-10-23 - Policy-gated authentication account failover
<!-- devlog-id: 2026-10-23-issue-2262-policy-gated-failover -->

### Result

- Added bounded auth-specific broker policy/capacity preflight and durable
  account-switch outcomes for Codex and Claude, retaining conversation and
  runtime/claim invariants without replaying prompts or tool effects.

### Why / context

- Capacity selectors alone do not authorize an auth switch. Fresh checks also
  run when queued Codex intents apply; only model health resets the
  consecutive-fault fence. Rollback guidance preserves strict receipt-store
  compatibility and uncertain effects.

### Evidence

- Four behavioral failures were captured before their fixes; 66 focused
  agent-session authentication tests passed with TMPDIR=/tmp. Strict file-size
  validation passed without baseline changes.

### Links

- <https://github.com/sympoies/nils-cli/issues/2262>
