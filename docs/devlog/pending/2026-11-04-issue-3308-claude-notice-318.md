## 2026-11-04 - Extend the Claude notice candidate range to 2.1.318
<!-- devlog-id: 2026-11-04-issue-3308-claude-notice-318 -->

### Result

- The native notice gate admits Claude Code 2.1.318 and refuses 2.1.319.

### Why / context

- The bounded external provider range requires review for each new release.

### Evidence

- The call-shape scan and plugin validation passed on integrity-verified
  official 2.1.318 packages; native acceptance remains pending before merge.

### Links

- <https://github.com/sympoies/nils-cli/issues/3308>
