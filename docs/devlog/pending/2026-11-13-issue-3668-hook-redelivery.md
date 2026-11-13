## 2026-11-13 - Recover webhook deliveries after a receiver switch
<!-- devlog-id: 2026-11-13-issue-3668-hook-redelivery -->

### Result

- Added repository and organization hook redelivery by status and delivery
  window, plus redeliver rows in the existing hook batch.

### Why / context

- Recovery must happen within GitHub retention. Redelivery is asynchronous, so
  read-back correlates a new attempt by GUID and reports pending without
  treating the original failure as a new result. Output only retains metadata;
  provider payloads, headers and free-form diagnostics are excluded.

### Evidence

- Portable stub-gh acceptance tests were written first; the pre-implementation
  run failed all ten cases because the command and batch action were absent.

### Links

- <https://github.com/sympoies/nils-cli/issues/3668>
