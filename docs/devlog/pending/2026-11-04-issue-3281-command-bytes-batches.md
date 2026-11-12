## 2026-11-04 - Run the runtime-command byte cases in small parallel batches
<!-- devlog-id: 2026-11-04-issue-3281-command-bytes-batches -->

### Result

- history_command_bytes::runtime_command_input_bytes_are_unchanged runs its 38
  independent real-binary sends in batches of at most 4, each with its own
  fixture, and names the failing case; every assertion is kept.

### Why / context

- The sends ran one after another and each paid the binary's production settle
  delay, so the test took about 40 s.

### Evidence

- Local, gate wrapper: 39.5 s before, 10.9 s after; 20 of 20 repeat runs passed
  (10.7 to 10.9 s).

### Links

- <https://github.com/sympoies/nils-cli/issues/3281>
