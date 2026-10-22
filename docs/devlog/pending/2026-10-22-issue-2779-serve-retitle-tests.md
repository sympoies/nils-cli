## 2026-10-22 - Extract serve retitle tests into a sibling module
<!-- devlog-id: 2026-10-22-issue-2779-serve-retitle-tests -->

### Result

- Move 25 retitle tests verbatim from serve.rs into a 1,865-line sibling test
  module; keep the implementation unchanged apart from test module wiring.

### Why / context

- Keep the extraction to one coherent topic below the 3,000-line test-file
  limit while reducing the existing serve test ratchet.

### Evidence

- Source reconstruction proves a verbatim move including its indentation and an
  unchanged 389-test normalized inline inventory. Strict file-size audit
  passes; the serve test baseline shrinks from 25,178 to 23,318 lines and
  implementation remains 11,763 lines.

- Hermetic focused nextest passed all 25 moved tests with no retries. Workspace
  formatting and a direct format check of the extracted module passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2779>

### Follow-ups

- Other serve test topics remain in the source file for separate extraction
  slices.
