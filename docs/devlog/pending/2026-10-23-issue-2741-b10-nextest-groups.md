## 2026-10-23 - Serialize resource-sensitive test classes
<!-- devlog-id: 2026-10-23-issue-2741-b10-nextest-groups -->

### Result

- The CI Nextest profile now serializes systemd/cgroup, CPU-sampling,
  kernel-tracking, and process-capacity test classes in dedicated one-thread
  groups. The dispatch latency benchmark is the only test that reserves the
  full pool.

### Why / context

- The full suite was capped by a small set of tests with shared kernel
  resources or wall-clock measurements. Per-class groups preserve those
  constraints while leaving unrelated tests parallel.

### Evidence

- Affected crates: cargo nextest run --profile ci -p nils-agent-session -p
  nils-agent-hook -p nils-git-cli (3,373 passed, 3 skipped, 1 retry-resolved
  flaky test; TMPDIR=/tmp for Unix-socket fixtures). Repository local-fast
  audits and Clippy passed; its workspace test execution was stopped after it
  began the full test matrix, as scoped for this batch.

### Links

- <https://github.com/sympoies/nils-cli/issues/2741>
