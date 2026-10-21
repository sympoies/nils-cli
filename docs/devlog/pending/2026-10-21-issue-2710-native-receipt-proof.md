## 2026-10-21 - Recover keyed native receipts after delayed input visibility
<!-- devlog-id: 2026-10-21-issue-2710-native-receipt-proof -->

### Result

- Keyed guarded Codex attempts retain private exact-turn proof for bounded
  read-only receipt confirmation after the initial history deadline.

### Why / context

- A matching native acknowledgment can precede input-history visibility.
  Keeping its runtime, provider, account and payload binding allows later
  confirmation while preserving strict admission and at-most-once dispatch.

### Evidence

- A controlled fake-provider regression failed on unknown versus submitted
  before production edits. The final 42 receipt tests and 90 guarded-send tests
  pass; original setup failures remain separate from meaningful regression
  evidence.

### Links

- <https://github.com/sympoies/nils-cli/issues/2710>

### Follow-ups

- Release-owned native provider qualification remains pending review and
  release; legacy attempts without retained proof remain unknown.
