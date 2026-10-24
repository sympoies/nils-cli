## 2026-10-24 - Address four agent-session cleanup follow-ups
<!-- devlog-id: 2026-10-24-issue-2861-cleanup-followups -->

### Result

- Keep non-path words in history summaries, exclude conflicting delete flags
  reciprocally in zsh completion, use deadline-based fixture prewarming, and
  stop restart fixture pane groups on drop or owner exit.

### Why / context

- Cleanup review identified over-redacted summaries and test fixtures whose
  timing or process lifetime could destabilize later runs.

### Evidence

- All four regression checks failed against the prior implementation; focused
  history, delete CLI, prewarm, and pane teardown checks passed after their fixes.

### Links

- <https://github.com/sympoies/nils-cli/issues/2861>
