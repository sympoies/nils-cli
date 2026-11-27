## 2026-11-26 - Admit relay-attested Console steward roots at the destination
<!-- devlog-id: 2026-11-26-console-attested-create-destination -->

### Result

- Destination session creation verifies the machine bearer, a dedicated
  credential, and a binding over the received body before persisting a steward
  root.

### Why / context

- The broker can verify the relay-attested role after a steward rotation while
  ordinary role labels remain descriptive.

### Evidence

- Destination fixtures cover authorization, root lineage, broker identity, and
  duplicate launch fencing. Released agent-session v1.43.0 returned HTTP 415
  with zero records and launches in a loopback probe.

### Links

- <https://github.com/sympoies/nils-cli/issues/4077>
