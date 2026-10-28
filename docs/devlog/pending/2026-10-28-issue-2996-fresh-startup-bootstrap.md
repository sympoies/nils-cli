## 2026-10-28 - Recover Codex startup without a conversation and serialize bootstrap account reads
<!-- devlog-id: 2026-10-28-issue-2996-fresh-startup-bootstrap -->

### Result

- Failed Codex startup with an explicitly queued account can restart fresh
  after native quiescence and verified runtime teardown; existing conversations
  retain exact resume semantics.

### Why / context

- A startup can fail before managed resume identity exists. Replacement TUI
  account discovery must also wait for credential application to avoid a
  provider bootstrap exit during workspace routing discovery.

### Evidence

- Baseline replay after PR #3036 produced four expected fixture failures: fresh
  launch admission, no-identity stop admission, bootstrap account discovery,
  and readiness. The implemented restart fixtures and bootstrap ordering
  fixture pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2996>
