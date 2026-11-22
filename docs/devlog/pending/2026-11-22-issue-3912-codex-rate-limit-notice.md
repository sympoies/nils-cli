## 2026-11-22 - Persist the Codex rate-limit model picker preference
<!-- devlog-id: 2026-11-22-issue-3912-codex-rate-limit-notice -->

### Result

- codex-cli can set the rate-limit model picker preference in a selected Codex
  home, and private launch projections carry the same preference.

### Why / context

- Codex records the opt-out in its notice table. Isolated exec retains its
  user-config bypass and sets the preference through a command-line override.

### Evidence

- Integration fixtures inspect the selected files and the configuration
  captured by both managed child processes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3912>
