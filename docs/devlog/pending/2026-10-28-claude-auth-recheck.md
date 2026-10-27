## 2026-10-28 - Settle Claude auth quarantine with a same-account switch
<!-- devlog-id: 2026-10-28-claude-auth-recheck -->

### Result

- A same-account switch on a quarantined Claude session re-runs the credential
  preflight and records credentials_refreshed, so input is admitted again; a
  refused preflight leaves the guard unchanged.

### Why / context

- No turn can be admitted while quarantined, so repaired credentials previously
  had no supported exit short of replacing the session.

### Evidence

- Regression tests failed before the change (repaired credential stayed
  quarantined) and pass after; unrepaired credentials and later failures stay
  refused.

### Links

- <https://github.com/sympoies/nils-cli/issues/3005>
