## 2026-10-30 - Keep worktree removal retryable when the destructive step is cut off
<!-- devlog-id: 2026-10-30-issue-3102-removal-retryable -->

### Result

- A worktree removal whose destructive git step fails or exceeds the bounded
  runner now restores any tracked content that went missing from the backup
  snapshot (or HEAD), keeps the target registered, and reports
  removal-remove-incomplete with the phase, the mutation state, and the
  recovery outcome.
- Every removal failure carries details.phase and details.worktree_mutation;
  fence and backup phases never mutate the target.

### Why / context

- The 15 second bounded runner also wrapped the destructive git worktree
  remove, so a slow deletion was killed mid-way and reported as a generic proof
  failure while tracked files, including the lockfile, were already gone.

### Evidence

- A disposable-repo fault injection reproduced exit 65
  removal-proof-unavailable with tracked files deleted and the worktree still
  registered; the new regressions failed on the deleted bytes first and pass
  with restored bytes, identical index and a successful rerun.

### Links

- <https://github.com/sympoies/nils-cli/issues/3102>

### Follow-ups

- A tree whose real deletion exceeds the bounded runner still cannot be
  removed; a destructive-phase deadline is a separate decision.
