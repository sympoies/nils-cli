## 2026-11-22 - Preserve delayed permission relay answers on macOS
<!-- devlog-id: 2026-11-22-issue-3861-permission-relay-stream -->

### Result

- The permission relay restores blocking I/O on accepted streams before setting
  the existing I/O deadlines.

### Why / context

- macOS can inherit the listener's nonblocking flag, causing a valid delayed
  answer to be dropped before the read timeout applies.

### Evidence

- A synthetic waiter returned the normal hook decision and acknowledgement in
  blocking mode, then closed before a delayed request with the accepted stream
  forced nonblocking on the baseline.

### Links

- <https://github.com/sympoies/nils-cli/issues/3861>
