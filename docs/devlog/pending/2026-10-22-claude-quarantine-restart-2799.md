## 2026-10-22 - Preserve operator restart during Claude auth quarantine
<!-- devlog-id: 2026-10-22-claude-quarantine-restart-2799 -->

### Result

- Permit the restart controller to submit its fixed provider exit command
  through a sealed internal capability while keeping external prompt input
  quarantined.
- Cover replacement quarantine before readiness and after readiness failure,
  activity-only quarantine when the incident store is full, printable keys,
  equal/lower generations with different publication snapshots, and rejection
  of unrelated broker capabilities, inconsistent publication evidence and
  checkpoints copied from another runtime incarnation, and the bounded
  unknown-effect selector sample with a separately retained total count.

### Why / context

- The shared input guard also blocked the lifecycle exit command, preventing
  operator restart with either force mode after authentication failure.

### Evidence

- Both new restart regressions first failed with auth-quarantined at admission
  on the previous PR head. All 46 focused restart/recovery, publication,
  startup-delivery and HTTP quarantine tests pass after merging current main,
  including exact exit bytes, both force modes and external input refusal.
- All six added admission regressions fail when their guards are removed;
  the generation boundary test fails when strict ordering permits equality.
  A non-empty unrelated capability list also fails when any capability is
  accepted; the exact capability check passes after restoration.
  Four independent publication-evidence and checkpoint-binding regressions
  fail with their matching guards removed. Production guards are restored
  before successful final validation.
- The follow-up selector-bound regression fails when the sample cap rises to
  nine. With the cap restored, it and the original two-operation recovery case
  pass; twelve unknown effects retain a total count of twelve and only eight
  metadata-only selectors. The external `/exit` assertion also fails under a
  text-only exception in both force modes, independently of Enter admission.
- Scoped Clippy with warnings denied, strict file-size audit, documentation
  audits, Markdown lint and devlog fragment validation pass. The combined
  `lib.rs` implementation baseline is measured as 12891 lines.

### Links

- <https://github.com/sympoies/nils-cli/pull/2799>
