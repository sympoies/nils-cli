## 2026-11-13 - Pin app-server diagnostic collection after FIFO EOF
<!-- devlog-id: 2026-11-13-issue-3671-collector-eof -->

### Result

- A gated FIFO regression directly checks the app-server collector after its
  writer closes.

### Why / context

- Existing HTTP fixtures hold stderr open briefly, so they do not isolate the
  FIFO reopen race. The original collector stalls in this fixture; the
  anonymous-pipe collector completes.

### Evidence

- Focused collector test passed; the original-source synthetic probe stalled
  and the repaired script retained the diagnostic. Under full-crate background
  load, startup cargo test passed 40/40 iterations and parallel nextest passed
  30/30 plus 20/20. One unrelated mailbox fixture failed in the background
  workload and remains separate from the startup evidence.

### Links

- <https://github.com/sympoies/nils-cli/pull/3674>
