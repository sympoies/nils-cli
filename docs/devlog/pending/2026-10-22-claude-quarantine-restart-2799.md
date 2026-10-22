## 2026-10-22 - Preserve operator restart during Claude auth quarantine
<!-- devlog-id: 2026-10-22-claude-quarantine-restart-2799 -->

### Result

- Permit the restart controller to submit its fixed provider exit command
  through a sealed internal capability while keeping external prompt input
  quarantined.
- Cover replacement quarantine before readiness and after readiness failure,
  activity-only quarantine when the incident store is full, printable keys,
  and equal/lower generations with different publication snapshots.

### Why / context

- The shared input guard also blocked the lifecycle exit command, preventing
  operator restart with either force mode after authentication failure.

### Evidence

- Both new restart regressions first failed with auth-quarantined at admission
  on the previous PR head. All 32 focused restart/recovery tests and three
  publication-generation tests pass, including exact exit bytes, both force
  modes and external input refusal.
- All six added admission regressions fail when their guards are removed;
  the generation boundary test fails when strict ordering permits equality.
  Production guards were restored before successful validation.
- Scoped Clippy with warnings denied, strict file-size audit, documentation
  audits, Markdown lint and devlog fragment validation pass.

### Links

- <https://github.com/sympoies/nils-cli/pull/2799>
