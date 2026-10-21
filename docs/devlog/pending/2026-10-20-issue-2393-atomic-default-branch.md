## 2026-10-20 - Default-branch commits install against the expected parent
<!-- devlog-id: 2026-10-20-issue-2393-atomic-default-branch -->

### Result

- The default-branch command creates a signed candidate and conditionally
  installs it without overwriting a competing branch advance.

### Why / context

- Checking HEAD before ordinary git commit allowed a later writer to become the
  parent. Post-installation failures now retain the installed identity and
  explicitly prohibit replay.

### Evidence

- A deterministic barrier regression failed before production edits; 39 focused
  default-branch integration tests pass, including hook vetoes and finalization
  recovery.

### Links

- <https://github.com/sympoies/nils-cli/issues/2393>
