## 2026-11-09 - Attest Codex mailbox and operator app-server notices
<!-- devlog-id: 2026-11-09-issue-3393-codex-notice-routes -->

### Result

- Mailbox controller start and steer and operator app-server notices reuse the
  bounded Codex attestation owner; only exact physical response/client-event
  proofs project as machine notices.

### Why / context

- Native acknowledgment and later text/timestamp matches cannot prove a unique
  machine row. Queue handoff now revalidates the consumed generation and holds
  the record lock through the actual native write, while terminal-only operator
  notices keep their existing unattested delivery.

### Evidence

- Meaningful RED retained on main for all three routes. Type checking, 75
  mailbox tests and 32 guarded notice tests passed, including local-socket
  receiver ingress and missing or ambiguous operator proof.

### Links

- <https://github.com/sympoies/nils-cli/issues/3393>

### Follow-ups

- Native Claude/Codex acceptance is scheduled separately before issue closeout.
