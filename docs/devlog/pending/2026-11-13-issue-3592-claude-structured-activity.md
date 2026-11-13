## 2026-11-13 - Replace Claude activity pane rules with structured observations
<!-- devlog-id: 2026-11-13-issue-3592-claude-structured-activity -->

### Result

- Claude activity no longer samples pane text or completes Stop through a quiet
  timer. Unavailable structured state projects unknown with a reason;
  qualifying Stop requires runtime/revision-bound post-Stop idle evidence.

### Why / context

- Structured state and hook correlation replace busy, idle and approval screen
  detection. Generic idle still cannot prove an interrupt or identify a
  manually denied approval, so conservative uncertainty and terminal input
  safety remain.

### Evidence

- The elapsed-time-only regression failed before production edits and is
  retained in test-first evidence. Structured fixture coverage includes
  unavailable causes, completion fences, cached fetch time, and continued or
  parallel Stop hooks; validation results accompany the PR.

### Links

- <https://github.com/sympoies/nils-cli/pull/3633>
