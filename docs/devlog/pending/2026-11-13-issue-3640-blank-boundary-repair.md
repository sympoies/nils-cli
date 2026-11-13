## 2026-11-13 - CLI usage normalization preserves blank section boundaries
<!-- devlog-id: 2026-11-13-issue-3640-blank-boundary-repair -->

### Result

- Usage line-ending normalization preserves blank section boundaries while
  collapsing transport CR padding.

### Why / context

- The review probe showed that dropping empty lines could attribute an
  unrelated context percentage to a rate-limit window. Two LF characters, or
  repeated CR without LF, therefore close the section.

### Evidence

- Four new synthetic blank-boundary tests failed before the repair: context
  attribution, heading gap, reset gap, and sanitized screen layout.

### Links

- <https://github.com/sympoies/nils-cli/pull/3664>
