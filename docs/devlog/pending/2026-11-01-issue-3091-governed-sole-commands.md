## 2026-11-01 - Align sourced shells with governed sole commands
<!-- devlog-id: 2026-11-01-issue-3091-governed-sole-commands -->

### Result

- Clarified the bounded worktree removal and current-main preparation forms
  alongside the existing authoring exception.

### Why / context

- Owner guards require these operations to run alone while ordinary development
  shells retain the repository environment.

### Evidence

- Issue #3091 records the conflicting guard requirements and the accepted
  command forms.

### Links

- <https://github.com/sympoies/nils-cli/issues/3091>
