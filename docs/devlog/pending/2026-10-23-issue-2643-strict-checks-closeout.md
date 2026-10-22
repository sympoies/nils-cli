## 2026-10-23 - Close strict check absence and visible blocker reporting gaps
<!-- devlog-id: 2026-10-23-issue-2643-strict-checks-closeout -->

### Result

- Strict one-shot checks return checks_pending when no checks were observed and
  include every effective-gate failure or pending row in the corresponding
  detail list.

### Why / context

- An empty snapshot cannot prove execution, and visible fallback failures need
  actionable detail even when no rows are configured as required.

### Evidence

- Both new regression fixtures failed before repair. After repair, 76 focused
  forge-cli tests, package Clippy with all targets and features and warnings
  denied, and the strict file-size audit passed. Cargo commands used a 6 GiB
  hard cap with swap disabled and four build jobs.

### Links

- <https://github.com/sympoies/nils-cli/pull/2808>
- <https://github.com/sympoies/nils-cli/issues/2643>
