## 2026-11-13 - Resolve test binaries under both Cargo test layouts
<!-- devlog-id: 2026-11-13-issue-3626-cargo-test-layout -->

### Result

- Library tests that need a built workspace binary find it under Cargo's
  pre-1.100 deps layout and under the 1.100 build/<package>/<hash>/out layout.

### Why / context

- Cargo 1.100 no longer places test executables in <profile>/deps, so the
  fallback that took the executable's grandparent as the profile directory
  missed the binary, and the test-only launch-executable branch keyed on a deps
  directory missed it too (114 agent-session tests failed on an unmodified
  tree). The shared helper now matches only the two known shapes, so a binary
  above the profile directory is never selected.

### Evidence

- Unit tests cover both layouts, an unknown layout and decoys in every ancestor
  of the executable. The full package-scoped suite passes under Cargo 1.99.1
  and 1.100.0.

### Links

- <https://github.com/sympoies/nils-cli/issues/3626>

### Follow-ups

- Other code that assumes a deps directory (a git-cli worker lookup, the CI
  target guard scan) is reported on the pull request for routing.
