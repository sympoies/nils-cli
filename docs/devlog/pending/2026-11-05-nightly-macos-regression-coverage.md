## 2026-11-05 - Document nightly macOS regression coverage
<!-- devlog-id: 2026-11-05-nightly-macos-regression-coverage -->

### Result

- Documented the existing scheduled macOS regression path and its daily runner
  budget.

### Why / context

- The existing nightly full validation already runs both partitions and tracks
  failures without adding per-merge runner cost.

### Evidence

- Scheduled run 41835167531 used about 55 macOS runner minutes; the two
  partition timeouts bound a scheduled run to 120 minutes.
