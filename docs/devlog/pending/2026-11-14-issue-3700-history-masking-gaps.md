## 2026-11-14 - Close history credential wrapper and query masking gaps
<!-- devlog-id: 2026-11-14-issue-3700-history-masking-gaps -->

### Result

- History masked and owner forms mask concatenated outer-quote suffixes,
  encoded query assignments, and relative URL query credentials without stray
  delimiters, retaining complete nested closing wrappers around relative URLs.
- Short query aliases match name components, preserving ordinary query names
  that merely contain those substrings.

### Why / context

- Closing an outer quote did not end concatenated secret text. Query
  recognition also skipped encoded assignment separators and relative URLs;
  repeated projection must preserve complete redaction markers.

### Evidence

- Five independent synthetic regressions failed on the unmodified main
  baseline; two nested-wrapper regressions also failed before their repair. All
  27 focused history-parts tests pass after the fix, including unchanged provider
  goldens and ordinary-text controls.

### Links

- <https://github.com/sympoies/nils-cli/issues/3700>
