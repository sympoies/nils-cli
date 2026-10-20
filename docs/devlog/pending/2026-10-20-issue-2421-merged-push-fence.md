## 2026-10-20 - Historical merged PR associations in workflow reruns
<!-- devlog-id: 2026-10-20-issue-2421-merged-push-fence -->

### Result

- Commit-discovered merged PR associations allow a different source head while
  open PR and embedded association head fences remain enforced.

### Why / context

- The GitHub commit association API returns the merged PR that introduced a
  default-branch commit; its source head differs from the merge or squash
  commit used by push runs.

### Evidence

- Before the fix, the new merged-push regression returned exit 65
  pr_head_mismatch: 10 tests passed and 1 failed. After the fix, all 11 focused
  run_rerun tests pass, including stale-open zero-POST refusal.

### Links

- <https://github.com/sympoies/nils-cli/pull/2699>
