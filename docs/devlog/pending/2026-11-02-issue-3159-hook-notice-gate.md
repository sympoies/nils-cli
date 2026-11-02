## 2026-11-02 - Keep writer-disabled Codex hooks from consuming notice generations
<!-- devlog-id: 2026-11-02-issue-3159-hook-notice-gate -->

### Result

- Codex hooks without enabled notice writers leave mailbox generations pending
  for the existing controller; the canonical mailbox.wake envelope and
  generation claims remain unchanged.

### Why / context

- A hook child can have a different rollout environment from the daemon.
  Treating the child environment as receiver capability allowed the fixed plain
  prompt to win the shared generation claim.

### Evidence

- An isolated native transport/idempotency probe passed before a cross-process
  authenticated-hook RED returned the exact fixed prompt. After the boundary
  fix, 12 notice and 22 notification tests passed, including unset and
  explicitly disabled hook environments.

### Links

- <https://github.com/sympoies/nils-cli/issues/3159>

### Follow-ups

- Field-row attribution and real provider/UI acceptance remain separate from
  hermetic source validation.
