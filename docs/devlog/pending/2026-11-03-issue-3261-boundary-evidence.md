## 2026-11-03 - Preserve session boundary probe evidence
<!-- devlog-id: 2026-11-03-issue-3261-boundary-evidence -->

### Result

- Guarded termination refusals carry the boundary evidence observed by
  verification.

### Why / context

- The verifier can retain an earlier live observation while a later probe is
  unavailable; reporting both preserves the existing fail-closed decision and
  makes it inspectable.

### Evidence

- Fixture tests cover recorded identities, PID reuse, cgroup outcomes, and
  changing verification observations.

### Links

- <https://github.com/sympoies/nils-cli/issues/3261>
