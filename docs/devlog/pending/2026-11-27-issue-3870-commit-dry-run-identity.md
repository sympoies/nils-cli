## 2026-11-27 - Check commit identity during semantic-commit dry runs
<!-- devlog-id: 2026-11-27-issue-3870-commit-dry-run-identity -->

### Result

- Plain commit dry runs use the same remote and URL selection as real commits.
  Ambiguous remotes report their candidates and selection guidance.

### Why / context

- A managed worktree with two unselected remotes could pass a dry run and fail
  the real commit before Git ran.

### Evidence

- The regression fixture reproduced the refusal and false dry-run success
  against the baseline. The repaired dry run leaves provider and identity audit
  state untouched.

### Links

- <https://github.com/sympoies/nils-cli/issues/3870>
