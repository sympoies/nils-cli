## 2026-11-24 - Add hosted macOS fixture diagnostics
<!-- devlog-id: 2026-11-24-hosted-macos-fixture-diagnostics -->

### Result

- A manual hosted macOS job runs pinned permission socket and short-root test
  fixtures, with retries disabled and optional LLVM instrumentation.

### Why / context

- The merge-group failures need timing and retained-reason evidence from the
  exact fixtures, including output from failed runs.

### Evidence

- Workflow lint passed, and the traced permission fixture and standalone socket
  comparator each passed 50 Linux iterations.

### Links

- <https://github.com/sympoies/nils-cli/issues/4023>
