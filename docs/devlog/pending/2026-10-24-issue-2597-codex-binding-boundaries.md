## 2026-10-24 - Fence literal Codex sends and validate repair accounts with the serve roster
<!-- devlog-id: 2026-10-24-issue-2597-codex-binding-boundaries -->

### Result

- Literal terminal text is refused before paste or keys when the selected Codex
  binding is not applied to the current runtime. Codex account switches prefer
  the valid serve-recorded broker command, and the daemon rechecks membership
  before promoting a queued repair.

### Why / context

- Authentication-recovery quarantine does not cover generic failed account
  bindings. Text without Enter must obey the binding boundary, and a
  caller-specific broker must not reject an account configured for serve or
  admit an unknown repair.

### Evidence

- Isolated CLI regressions failed before production edits and pass afterward. A
  separate failing-first daemon regression proves unknown repair accounts leave
  the binding and queued intent unchanged; existing account, roster,
  incarnation, input-fence and recovery suites pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2597>
