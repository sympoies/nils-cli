## 2026-10-26 - Deterministic remote burst admission fixture
<!-- devlog-id: 2026-10-26-issue-2944-burst-clock -->

### Result

- Freeze admission milliseconds in the remote burst fixture while retaining the
  production clock and rate limits.

### Why / context

- Filesystem latency can exceed the one-second burst window and falsely admit
  the eleventh fixture message.

### Evidence

- A deliberate 1100 ms delay reproduced the failure before the clock seam;
  all 37 focused remote-receive tests passed after freezing the fixture clock.

### Links

- <https://github.com/sympoies/nils-cli/issues/2944>
