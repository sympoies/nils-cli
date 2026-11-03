## 2026-11-03 - Extend the Claude notice candidate range to 2.1.317
<!-- devlog-id: 2026-11-03-issue-3270-claude-notice-317 -->

### Result

- The native notice version gate admits Claude Code 2.1.317 and refuses
  2.1.318.

### Why / context

- The reviewed range remains bounded and does not widen at runtime.

### Evidence

- The call-shape scan and plugin validation passed against the
  integrity-verified official 2.1.317 package; native acceptance remains
  pending before merge.

### Links

- <https://github.com/sympoies/nils-cli/issues/3270>
