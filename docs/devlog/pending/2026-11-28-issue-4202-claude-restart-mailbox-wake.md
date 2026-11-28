## 2026-11-28 - Restore mailbox wakes after Claude replacement
<!-- devlog-id: 2026-11-28-issue-4202-claude-restart-mailbox-wake -->

### Result

- Retain current-incarnation SessionStart readiness when the native Claude
  listing is unavailable.
- Allow one guarded read-mail line before the replacement’s first hook when
  fresh native idle and an empty composer are both verified.

### Why / context

- A restarted terminal can be idle without a prior turn outcome. A native
  listing failure could also mask otherwise valid startup readiness.

### Evidence

- Fake-provider regressions reproduced both queued wakes before the fix. The
  shared-guard preview passed the restart, draft preservation, and Claude and
  Codex wake cases.

### Links

- <https://github.com/sympoies/nils-cli/issues/4202>
