## 2026-10-21 - Patch certificate verification and bundled WebSocket trust dependencies
<!-- devlog-id: 2026-10-21-issue-2708-certificate-trust-dependencies -->

### Result

- Updated the certificate verifier and both bundled root packages to patched
  releases; removed the obsolete WebSocket root wrapper while retaining bundled
  Mozilla trust.

### Why / context

- Tungstenite still selects the affected 0.26 root package. Its supported
  stream handshake API permits current bundled roots without substituting the
  system trust store or weakening verification.

### Evidence

- The existing dependency audit failed on four package instances before edits
  and passed after repair against the same official advisory database revision.
  All 366 affected tests passed, including trusted TLS, unknown issuer,
  hostname mismatch, SNI, headers, redirects and timeout cases.

### Links

- <https://github.com/sympoies/nils-cli/issues/2708>
