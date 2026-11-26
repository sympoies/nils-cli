## 2026-11-26 - Serve authenticated Console resume-move destinations
<!-- devlog-id: 2026-11-26-console-resume-move-destination -->

### Result

- Added the destination start and read-only status routes, request-time
  capability discovery, and bearer-gated move lineage.

### Why / context

- The relay delegates admission to the native destination engine and reconciles
  possible dispatch through durable outcomes.

### Evidence

- Loopback contract fixtures exercise typed refusals, capability, replay, and
  read-only status with synthetic signed proofs.

### Links

- <https://github.com/sympoies/nils-cli/issues/4059>
