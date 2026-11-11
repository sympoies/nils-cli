## 2026-11-04 - advisory slow-test ratchet and nightly lane timing report
<!-- devlog-id: 2026-11-04-issue-3280-slow-test-ratchet -->

### Result

- An advisory step names tests of 10 s or more that are missing from
  scripts/ci/slow-test-baseline.tsv; the baseline may only shrink. The nightly
  tracker reports per-platform test count, nextest wall, build time and tests
  of 1 s or more, and flags a lane budget breach (Linux 25 min, macOS 40 min).

### Why / context

- Tests of 1 s or more doubled in a month and nothing stopped new slow tests
  from landing.

### Evidence

- Baseline of 89 tests of 6 s or more seeded from the Linux JUnit of a green main run after
  the speedup batch. No test time is added or removed by this change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3280>
