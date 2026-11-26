## 2026-11-27 - Gate worktree cleanup on caller-selected local predicates
<!-- devlog-id: 2026-11-27-issue-2362-conditional-worktree-removal -->

### Result

- Managed worktree removal can require an exact HEAD, a clean checkout, and
  ancestry to a selected local ref.

### Why / context

- Cleanup callers can have their eligibility sample rechecked under the same
  fences that protect removal, before backup or lease release.

### Evidence

- Deterministic git-cli integration fixtures cover HEAD changes, staged and
  unstaged edits, untracked content, local ancestry, and fail-closed retention.

### Links

- <https://github.com/sympoies/nils-cli/issues/2362>
