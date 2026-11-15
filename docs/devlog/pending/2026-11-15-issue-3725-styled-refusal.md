## 2026-11-15 - Stabilize styled composer refusal fixture
<!-- devlog-id: 2026-11-15-issue-3725-styled-refusal -->

### Result

- The synthetic refusal test uses an immediate styled capture, and a unit test
  injects its retry window to exercise the intended observation.

### Why / context

- Subprocess scheduling could exhaust the retry deadline before the fixture
  reported its observation.

### Evidence

- The focused test and 32 concurrent runs on one CPU passed; the separate retry
  deadline test passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3725>
