## 2026-11-03 - Split macOS release validation into two partitions
<!-- devlog-id: 2026-11-03-macos-partition-split -->

### Result

- Full-validation macOS tests run as two nextest partitions on separate
  runners. `test_macos` now merges their LCOV reports and enforces the 85% line
  floor, and the release-only repeated audits, script tests, fmt and clippy no
  longer run on macOS.

### Why / context

- The macOS job was the critical path of a release queue entry (about 60
  minutes). Linux full validation runs the removed checks on the same commit
  and `macos_clippy` lints the macOS build.

### Evidence

- Workflow contract tests and the merge-lcov unit test pass. The real duration
  is unmeasured until the next release group runs.

### Links

- <https://github.com/sympoies/nils-cli/issues/3277>
