## 2026-10-23 - Retain non-Linux mapped hardlink holders
<!-- devlog-id: 2026-10-23-non-linux-mapped-hardlink-holders -->

### Result

- Use bounded lsof target inode selection and refuse incomplete probes; retain
  mapped hardlink aliases even when Darwin reports an outside name.

### Why / context

- A path-only lsof filter can miss mapped vnodes with ambiguous hardlink names.
  Python mmap also duplicates its descriptor, so holder tests now create a
  native mmap and close the only descriptor before signaling readiness.

### Evidence

- Captured a failing selected-alias parser regression before the fix. All 14
  focused removal unit tests, 53 worktree integration tests, and scoped Clippy
  passed with a 6 GiB hard memory cap and no swap. The standard local gate passed
  all 488 git-cli tests with no skips, using a short temporary root for Unix
  socket fixtures. macOS runtime acceptance remains CI-owned.

### Links

- <https://github.com/sympoies/nils-cli/issues/2790>
- <https://github.com/sympoies/nils-cli/pull/2788>
