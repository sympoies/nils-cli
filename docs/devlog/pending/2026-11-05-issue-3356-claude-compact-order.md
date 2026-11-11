## 2026-11-05 - Correct Claude compact history ordering
<!-- devlog-id: 2026-11-05-issue-3356-claude-compact-order -->

### Result

- Associate the metadata caveat with its compact pair and emit ascending stable
  row IDs.

### Why / context

- Claude Code 2.1.317 places a metadata caveat between the summary and command
  echo.

### Evidence

- Provider-shaped regression reproduced the inverted order before the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3356>
