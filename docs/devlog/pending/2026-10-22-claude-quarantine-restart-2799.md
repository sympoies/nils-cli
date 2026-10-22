## 2026-10-22 - Preserve operator restart during Claude auth quarantine
<!-- devlog-id: 2026-10-22-claude-quarantine-restart-2799 -->

### Result

- Permit the restart controller to submit its fixed provider exit command
  through a sealed internal capability while keeping external prompt input
  quarantined.

### Why / context

- The shared input guard also blocked the lifecycle exit command, preventing
  operator restart with either force mode after authentication failure.

### Evidence

- Both new restart regressions first failed with auth-quarantined at admission
  on the previous PR head. All 26 focused restart/recovery tests pass after the
  fix, including exact exit bytes, both force modes and external input refusal.
- Scoped Clippy with warnings denied, strict file-size audit, documentation
  audits, Markdown lint and devlog fragment validation pass.

### Links

- <https://github.com/sympoies/nils-cli/pull/2799>
