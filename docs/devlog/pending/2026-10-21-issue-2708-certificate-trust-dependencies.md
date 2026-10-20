## 2026-10-21 - Patch certificate verification and bundled WebSocket trust dependencies
<!-- devlog-id: 2026-10-21-issue-2708-certificate-trust-dependencies -->

### Result

- Updated the certificate verifier and both bundled root packages to patched
  releases; removed the obsolete WebSocket root wrapper while retaining bundled
  Mozilla trust.
- Corrected a recurring activity-refresh cadence test to observe the production
  refresh-start boundary while retaining its strict 250 ms minimum.
- Renewed the full-mailbox test's synthetic broker heartbeat per authenticated
  operation while retaining all 256 real acknowledgements and quota assertions.

### Why / context

- Tungstenite still selects the affected 0.26 root package. Its supported
  stream handshake API permits current bundled roots without substituting the
  system trust store or weakening verification.
- Collector entry follows blocking-pool dispatch and can underestimate the
  interval between refresh-operation starts when dispatch delays differ.
- The mailbox fixture starts no heartbeat updater. Its single seed beat can
  expire during a long acknowledgement loop even though quota behavior is correct.

### Evidence

- The existing dependency audit failed on four package instances before edits
  and passed after repair against the same official advisory database revision.
  All 366 affected tests passed, including trusted TLS, unknown issuer,
  hostname mismatch, SNI, headers, redirects and timeout cases.
- A controlled dispatch-delay probe observed producer starts 275 ms apart and
  collector entries 175 ms apart. The unchanged collector assertion failed;
  the corrected real-time test and all 26 affected activity-broker tests passed.
  Production scheduling, timing limits, retries and thread reservations remain
  unchanged. The original failed workspace gate remains retained evidence.
- Aging only the fixture heartbeat before the second acknowledgement reproduced
  the exact stale-authority error. With explicit renewal, the same probe passed
  all 256 acknowledgements and final delivery checks; production freshness and
  the 30-second limit remain unchanged.

### Links

- <https://github.com/sympoies/nils-cli/issues/2708>
- <https://github.com/sympoies/nils-cli/issues/1705>
- <https://github.com/sympoies/nils-cli/issues/2599>
- <https://github.com/sympoies/nils-cli/issues/2633>
