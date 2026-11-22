## 2026-11-22 - Bind fresh Console Codex resume identity to its attachment
<!-- devlog-id: 2026-11-22-console-codex-resume-binding -->

### Result

- Require a loaded thread matching the current app-server attachment before
  writing fresh resume metadata.

### Why / context

- A concurrent rollout in the same working directory can satisfy the standalone
  history heuristic.

### Evidence

- Synthetic nextest regression cases failed before the fix. All 20 focused
  binding, hook, and controller tests passed after the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3944>
