## 2026-11-23 - Defer idle Codex mailbox wakes during an operator draft
<!-- devlog-id: 2026-11-23-codex-mailbox-draft-deferral -->

### Result

- Idle native mailbox wakes check the bound composer before claim and at the
  write boundary. An occupied composer preserves the draft and keeps the
  generation queued with recipient-composer-not-empty.

### Why / context

- The first-mailbox-wake guard covered rollout discovery, while an idle
  recipient with a readable history baseline could start a turn under a draft.

### Evidence

- A socket-backed fixture reproduced turn/start on the baseline with a draft
  present and with a draft introduced after the first snapshot.

### Links

- <https://github.com/sympoies/nils-cli/issues/3955>
