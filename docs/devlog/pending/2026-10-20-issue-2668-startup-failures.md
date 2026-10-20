## 2026-10-20 - Report observed session startup failures to create callers
<!-- devlog-id: 2026-10-20-issue-2668-startup-failures -->

### Result

- Fresh session startup exits and recognized trust or setup prompts return a
  typed start-failed response with bounded diagnostics and conservative retry
  metadata.
- Diagnostic redaction precedes response clipping, and a provider exit during
  initial prompt delivery retains the concrete startup failure.

### Why / context

- A stopped provider could previously be returned as a successful create, while
  Console forwarding discarded failure evidence and left reconciliation
  ambiguous.

### Evidence

- Fake-provider reproductions preceded failing CLI, HTTP and Console
  regressions. Thirteen startup cases and the Console forwarding and
  reconciliation regression passed after the review repairs, including
  credential-boundary and delayed initial-prompt exit cases.
- Isolated build artifacts repeated the startup and Console contracts after a
  shared executable resolved to another revision; all assertions were retained.

### Links

- <https://github.com/sympoies/nils-cli/issues/2668>
