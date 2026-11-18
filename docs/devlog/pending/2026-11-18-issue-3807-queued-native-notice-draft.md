## 2026-11-18 - Keep queued native notices pending across Claude drafts
<!-- devlog-id: 2026-11-18-issue-3807-queued-native-notice-draft -->

### Result

- The notice queue keeps retrying the authenticated preclaim composer refusal
  with the original idempotency key and delivers after the user clears the draft.

### Why / context

- The native bridge reports an occupied composer before any text or key is
  consumed, so the existing guarded queue can safely try fresh admission.

### Evidence

- The synthetic busy-to-idle fixture reproduced the original refusal, then
  passed with the draft and cursor preserved, one native submission after
  clearing, and duplicate-key rejection.

### Links

- <https://github.com/sympoies/nils-cli/issues/3807>
