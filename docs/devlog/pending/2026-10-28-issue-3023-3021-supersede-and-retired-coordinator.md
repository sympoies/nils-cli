## 2026-10-28 - Supersede PRs and retired-coordinator proof
<!-- devlog-id: 2026-10-28-issue-3023-3021-supersede-and-retired-coordinator -->

### Result

- Added forge-cli pr create --supersedes, which verifies the old PR's
  test-first record at the old head and admits a new head that differs only by
  default-branch merges. Closed agent-session board rows now always carry
  last_terminal_activity_at.

### Why / context

- A deleted coordinator left no terminal-activity proof and a replacement PR
  needed a fresh record even when its source was unchanged, so teams mislabeled
  kinds or stalled. Source equivalence uses ancestry, no non-default-branch
  commits and an equal patch id against the default branch.

### Evidence

- New tests failed first and pass: supersede accept, three source-changed
  refusals, record-head mismatch and kind scope, plus the closed-record
  timestamp case. Focused crate tests and clippy passed; hosted CI is the gate.

### Links

- <https://github.com/sympoies/nils-cli/issues/3023>
- <https://github.com/sympoies/nils-cli/issues/3021>
