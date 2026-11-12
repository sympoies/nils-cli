## 2026-11-09 - Retry failed Codex bindings after credential rotation
<!-- devlog-id: 2026-11-09-issue-3168-codex-replica-apply -->

### Result

- Explicit resume and native-idle restart can reapply the failed selected
  account using the current broker credential without requiring a queued
  replacement. Queued replacement recovery retains priority.
- External-auth apply records closed-set broker and login failure causes; a
  generic daemon completion preserves the control diagnosis.

### Why / context

- Resume skipped every failed binding, and restart exit admission required a
  queued replacement. A synced credential could therefore remain unused while
  the binding stayed failed. Login failures also lost their branch identity.

### Evidence

- Retained RED results: relaunch left the binding failed instead of pending; a
  synthetic rotated token reached the login RPC but rejection became
  apply_failed; native-idle same-account restart refused with
  codex-account-not-bound.

### Links

- <https://github.com/sympoies/nils-cli/issues/3168>
