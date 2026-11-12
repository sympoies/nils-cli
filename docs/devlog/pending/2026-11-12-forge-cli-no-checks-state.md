## 2026-11-12 - Report an empty check set as no-checks, never success
<!-- devlog-id: 2026-11-12-forge-cli-no-checks-state -->

### Result

- forge-cli pr checks and everything built on pr_checks::aggregate() report
  no-checks for a head with zero checks instead of success; the old behavior is
  removed. Strict reads fail with checks_not_registered; waits keep polling and
  expire with that code.
- On GitHub, a workflow run of the head that is queued or running with no jobs
  yet is a pending row naming the run id and age.
- Repositories with genuinely no checks keep the existing explicit opt-ins
  (--allow-no-checks, [checks] none = true).

### Why / context

- A PR whose only CI run stayed queued with no jobs read as success with zero
  gates, so delivery monitors labeled it green.

### Evidence

- Test-first record retained with the red run; focused forge-cli tests and the
  local-fast gate pass after the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3561>
