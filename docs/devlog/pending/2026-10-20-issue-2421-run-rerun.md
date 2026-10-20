## 2026-10-20 - Fenced GitHub workflow reruns
<!-- devlog-id: 2026-10-20-issue-2421-run-rerun -->

### Result

- Added identity-scoped whole-run, failed-only, and selected-job reruns with
  current PR-head fencing and attempt/job receipts.

### Why / context

- Infrastructure-cancelled CI attempts need a bounded rerun without no-op
  commits or raw provider write fallback; delayed receipt reads must never
  replay the accepted write.

### Evidence

- Nine hermetic run_rerun integration tests pass after six missing-command
  regressions were captured before implementation.

### Links

- <https://github.com/sympoies/nils-cli/issues/2421>
