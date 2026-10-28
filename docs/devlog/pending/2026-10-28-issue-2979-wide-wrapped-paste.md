## 2026-10-28 - Guarded send proves wide-character wrapped Claude pastes
<!-- devlog-id: 2026-10-28-issue-2979-wide-wrapped-paste -->

### Result

- Claude's wrapped-paste proof now accepts any text of one- or two-cell
  characters, counting cursor steps in characters and endpoints in terminal
  cells, so a wrapped CJK line is recognised and submitted exactly once instead
  of timing out at the submit stage. Cleanup uses the same counts.

### Why / context

- The proof and cleanup were gated on printable ASCII and counted bytes, so a
  wrapped wide-character paste could never be proven; observation retried until
  the send deadline and the paste stayed in the composer. Zero-width and
  control characters stay unobservable. Wrapped Codex pastes still lack a proof
  and remain a fail-closed receipt.

### Evidence

- Fixture regression tests failed before the change with
  send-guard-observation-timeout at stage=submit and pass after; guarded_send
  suite and unit tests green.

### Links

- <https://github.com/sympoies/nils-cli/issues/2979>
