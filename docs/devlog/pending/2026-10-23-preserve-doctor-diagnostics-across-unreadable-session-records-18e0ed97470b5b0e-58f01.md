## 2026-10-23 - Preserve doctor diagnostics across unreadable session records
<!-- devlog-id: 2026-10-23-preserve-doctor-diagnostics-across-unreadable-session-records-18e0ed97470b5b0e-58f01 -->

### Result

- Doctor inspects pinned helpers through validated read-only session records,
  skipping unreadable entries while retaining healthy session findings and
  provider health.

### Why / context

- An aggregate session-list error previously made a corrupt neighboring record
  hide all doctor output; startup reconciliation also does not belong in this
  best-effort scan.

### Evidence

- The mixed corrupt/healthy-session regression failed before the repair and
  passes after it, including unchanged bytes for a stale startup record.
- Focused doctor suite passed 11 tests. Provider filtering, absolute paths, and
  permission-error handling each have a regression test that fails under its
  corresponding guard mutation.

### Links

- <https://github.com/sympoies/nils-cli/pull/2806>
- <https://github.com/sympoies/nils-cli/issues/2672>
