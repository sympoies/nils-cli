## 2026-11-24 - Recover Claude sessions after shared credential publication
<!-- devlog-id: 2026-11-24-issue-4008-claude-shared-refresh -->

### Result

- Allow one guarded same-account resume for revoked Claude sessions when the
  broker verifies a newer publication within 120 seconds.

### Why / context

- Publication can finish after the previous 30-second window. A fresh process
  can use the shared account directory while the lifecycle controller preserves
  the original failure and avoids replaying unknown effects.

### Evidence

- The synthetic regression reproduced quarantine after late publication on the
  baseline. The shared-account cases exercise two revoked siblings,
  wrong-account publication, and deadline expiry through production hook
  dispatch.

### Links

- <https://github.com/sympoies/nils-cli/issues/4008>
