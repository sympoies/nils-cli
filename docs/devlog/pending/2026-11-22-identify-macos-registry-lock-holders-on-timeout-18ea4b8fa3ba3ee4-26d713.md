## 2026-11-22 - Identify macOS registry lock holders on timeout
<!-- devlog-id: 2026-11-22-identify-macos-registry-lock-holders-on-timeout-18ea4b8fa3ba3ee4-26d713 -->

### Result

- Registry lock timeout diagnostics can identify active macOS holders by their
  kernel process start time.

### Why / context

- A missing or mismatched start time produces a stale descriptor so a reused
  PID cannot be attributed to a prior holder.

### Evidence

- All three targeted mutations failed their regression tests. The focused
  lock-holder suite passed on Linux.

### Links

- <https://github.com/sympoies/nils-cli/issues/3619>
