## 2026-11-13 - CLI usage parser normalizes line runs and selects newest frame
<!-- devlog-id: 2026-11-13-issue-3640-usage-parser-followups -->

### Result

- The usage parser collapses CR/LF runs, uses the newest screen without stale
  windows or resets, and pins collapsed percentages as unavailable.

### Why / context

- PTY captures may contain mixed line endings and repeated screens; stale reset
  values must not survive a refresh.

### Evidence

- Retained pre-fix parser run: 7 passed, 2 failed for CR/LF runs and stale
  reset reuse.

### Links

- <https://github.com/sympoies/nils-cli/issues/3640>
