## 2026-11-14 - Make unchanged-publication Claude auth refusal actionable
<!-- devlog-id: 2026-11-14-issue-3684-same-account-auth-restart -->

### Result

- Unchanged verified publications retain quarantine with a provider-proof
  refusal and same-account restart guidance; the ready replacement no longer
  inherits the predecessor authentication marker.

### Why / context

- Materialization and broker validity do not independently prove provider
  success. A synthetic lifecycle regression also showed that merely
  recommending restart left the replacement fenced by historical activity;
  original incident and unknown-effect evidence remain intact.

### Evidence

- Retained pre-edit RED: 5 passed and 1 failed; restart-path RED: 39 passed and
  1 failed. After the fix, all 40 focused auth recovery tests passed on the
  pinned toolchain.

### Links

- <https://github.com/sympoies/nils-cli/issues/3684>
