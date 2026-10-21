## 2026-10-20 - Add scoped credentials for supervised session workers
<!-- devlog-id: 2026-10-20-add-scoped-credentials-for-supervised-session-workers-18e01c34f50ac5cf-3b535f -->

### Result

- Managed callers can mint expiring operation and recipient scopes, revoke
  worker delegations, and audit admission without copying runtime credentials.

### Why / context

- Supervised workers need bounded session authority while runtime replacement
  and caller shutdown must revoke future operations.

### Evidence

- Delegation unit regressions and loopback daemon acceptance passed for Codex
  and Claude; scoped local delivery, remote submission, revocation and
  replacement refusal exercised.

### Links

- <https://github.com/sympoies/nils-cli/issues/2412>
