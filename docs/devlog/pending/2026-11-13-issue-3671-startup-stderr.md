## 2026-11-13 - Drain startup diagnostics through anonymous pipes
<!-- devlog-id: 2026-11-13-issue-3671-startup-stderr -->

### Result

- Both startup stderr collectors feed their bounded tail command through an
  anonymous pipe.

### Why / context

- A synthetic trace with uutils coreutils 0.10.0 showed tail reopening FIFO
  stdin after the writer exited, blocking collection until runtime cleanup
  discarded the diagnostic. Preserve capture bounds and exit detection rather
  than extending timeouts.

### Evidence

- The unchanged startup_failure suite failed iteration 1 of a 20-iteration
  stress run before the fix: 12 passed, one omitted stderr_tail. The trace
  independently retained exit code 23 and an empty diagnostic.

### Links

- <https://github.com/sympoies/nils-cli/issues/3671>
