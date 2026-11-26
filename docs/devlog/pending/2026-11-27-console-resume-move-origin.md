## 2026-11-27 - Route Console resume moves through the managed origin
<!-- devlog-id: 2026-11-27-console-resume-move-origin -->

### Result

- Managed callers can start an exact provider move through Console after v1
  capability discovery. The origin retains a private dispatch fence and reuses
  the original request for read-only reconciliation.
- The origin scopes receipts and operation bindings to the authenticated
  caller, bounds each caller's unresolved and retained moves, compacts
  terminal receipts to their fence and reply, and refuses future-dated
  operation IDs. A binding conflict reports an unresolved outcome.

### Why / context

- An ambiguous response must preserve the original move outcome across origin
  daemon restarts and cannot authorize a second destination launch.

### Evidence

- Five loopback contract fixtures fail at the former refusal on the baseline.
  The focused move and wire-contract run passes 15 tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/4059>

### Follow-ups

- Enable the relay capability for the configured route after the origin and
  destination daemons are updated.
