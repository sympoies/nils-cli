## 2026-10-21 - Resolve the owning repository for absolute worktree removal
<!-- devlog-id: 2026-10-21-issue-2715-worktree-remove-owning-repo -->

### Result

- git-cli worktree remove now resolves the owning repository from an absolute
  linked-worktree path, so a session rooted in one repository can remove a
  finished managed worktree of another; the JSON receipt reports repo_root.

### Why / context

- The repository was resolved from the caller working directory, so a
  cross-repository absolute path failed with worktree-not-found. All removal
  fences still run unchanged against the resolved repository.

### Evidence

- cargo test -p nils-git-cli --test integration worktree: two new
  cross-repository tests failed before the change (worktree-not-found) and pass
  after; the existing worktree and removal tests still pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2715>

### Follow-ups

- Worktrees under a previous managed root (#2578) and conditional eligibility
  (#2362) are unchanged and tracked separately.
