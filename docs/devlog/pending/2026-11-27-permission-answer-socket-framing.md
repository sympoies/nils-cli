## 2026-11-27 - Frame permission socket requests without waiting for EOF
<!-- devlog-id: 2026-11-27-permission-answer-socket-framing -->

### Result

- Require a bounded newline-terminated JSON request on the private permission
  socket and re-enable the delayed-answer test on macOS.

### Why / context

- A blocking Unix socket on macOS timed out waiting for EOF even after the
  sender finished the request and half-closed its write side.

### Evidence

- A local accepted-socket probe received every request byte before the EOF wait
  expired. The synchronized delayed-answer regression failed before the fix and
  passed 50 consecutive macOS runs with retries disabled after the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/4014>
