## 2026-10-20 - Report observed session startup failures to create callers
<!-- devlog-id: 2026-10-20-issue-2668-startup-failures -->

### Result

- Fresh session startup exits and recognized trust or setup prompts return a
  typed start-failed response with bounded diagnostics and conservative retry
  metadata.

### Why / context

- A stopped provider could previously be returned as a successful create, while
  Console forwarding discarded failure evidence and left reconciliation
  ambiguous.

### Evidence

- Fake-provider reproductions preceded failing CLI, HTTP and Console
  regressions. Ten startup cases and the full workspace local-fast gate passed
  on signed head 7855640b5b15560ee3c41cbd58251bbdd0db2e9f before this
  layout-only entry migration.

### Links

- <https://github.com/sympoies/nils-cli/issues/2668>
