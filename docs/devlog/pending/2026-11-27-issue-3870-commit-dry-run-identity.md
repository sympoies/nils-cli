## 2026-11-27 - Check commit identity during semantic-commit dry runs
<!-- devlog-id: 2026-11-27-issue-3870-commit-dry-run-identity -->

### Result

- Dry-run and real commits share forge identity admission. Ambiguous remotes
  report their candidates and selection guidance.

### Why / context

- A managed worktree with two unselected remotes could pass a dry run and fail
  the real commit before Git ran.

### Evidence

- The regression fixture reproduced the refusal and false dry-run success
  against the baseline.

### Links

- <https://github.com/sympoies/nils-cli/issues/3870>
