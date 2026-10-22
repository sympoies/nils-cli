## 2026-10-23 - Require macOS compile and Clippy in every merge group
<!-- devlog-id: 2026-10-23-issue-2809-macos-clippy -->

### Result

- Added affected-package macOS Clippy with all targets and features, including
  integration tests, and a fail-closed dependency from the required Linux
  check.

### Why / context

- Platform-specific test-helper warnings must block ordinary merges before
  reaching the release queue; full macOS tests and coverage remain
  release/nightly gates.

### Evidence

- Test-first queue scope fixtures fail before implementation and pass after
  adding the macOS consumer; delivery DAG fixtures pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2809>
