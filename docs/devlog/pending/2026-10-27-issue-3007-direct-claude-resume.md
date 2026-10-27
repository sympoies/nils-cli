## 2026-10-27 - Create new managed records from exact Claude conversations
<!-- devlog-id: 2026-10-27-issue-3007-direct-claude-resume -->

### Result

- Added direct Claude start --resume and exact read-only resume-preflight v1
  with terminal typed refusals and source writer guards.

### Why / context

- Reuse the import resolver and ordinary start initialization so conversation
  identity, original account, ownership, roles and prompts stay consistent.
  Deleted and quarantined sources remain refused rather than guessing
  authority.

### Evidence

- Two new CLI cases failed on the missing flag first; seven resume fixtures, 47
  affected resume regressions, the import lifecycle test and crate Clippy
  passed. Console, Codex and disposable-provider acceptance remain separate.

### Links

- <https://github.com/sympoies/nils-cli/issues/3007>
