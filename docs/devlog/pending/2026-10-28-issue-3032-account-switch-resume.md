## 2026-10-28 - Keep failed Claude account switches recoverable
<!-- devlog-id: 2026-10-28-issue-3032-account-switch-resume -->

### Result

- Explicit bound-account new-record resume can abandon pending Claude account
  switches; in-place retries preflight retained queued accounts.
- Account switches require verified credential publication and recheck turn
  activity before stopping.

### Why / context

- Failed switch intents were rejected as unknown by new-record resume, and
  directory materialization alone did not verify the next credentials.

### Evidence

- Four baseline regression failures reproduced; focused account-switch suite
  passed all 15 tests after the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3032>
