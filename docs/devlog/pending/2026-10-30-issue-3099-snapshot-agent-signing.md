## 2026-10-30 - Use the selected commit identity for worktree snapshots
<!-- devlog-id: 2026-10-30-issue-3099-snapshot-agent-signing -->

### Result

- Worktree removal snapshot commits use the existing forge commit authorization
  and command-local signing identity, with bounded execution, redaction and
  completion auditing.
- Detached snapshots use the existing identity owner's repository remote
  selection while preserving attached branch precedence and ambiguous-target
  refusal.

### Why / context

- An unavailable ambient signing key could prevent preserving staged work even
  when the selected agent signer was usable. Authorization or signing refusal
  retains the worktree and original index with identity recovery guidance.

### Evidence

- The pre-fix regression returned removal-backup-failed for commit signing; the
  corrected regression verifies the selected backup signer and restores the
  staged bytes. A detached-worktree regression first reproduced identity target
  refusal, then passed with verified selected signing and restored bytes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3099>

### Follow-ups

- Rebase and continuation support remain a separate delivery.
