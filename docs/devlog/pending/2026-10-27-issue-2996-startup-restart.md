## 2026-10-27 - Codex startup account recovery through restart
<!-- devlog-id: 2026-10-27-issue-2996-startup-restart -->

### Result

- A native idle Codex session with a failed binding and explicit queued account
  switch can request a normal restart exit while lifecycle activity is still
  starting; a disposable-session idle acceptance recipe uses normalized events
  without a model turn.

### Why / context

- Replacement-runtime account recovery already existed, but exit admission
  rejected the failed binding before it could run. The exception belongs only
  to the sealed restart exit after native identity, idle, composer and
  concurrency admission; ordinary input remains fenced.

### Evidence

- The new native-protocol admission regression failed before the fix with
  codex-account-not-bound (exit 101). Focused and local-fast validation are
  recorded in the linked delivery PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/2996>
