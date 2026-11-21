## 2026-11-22 - Read GitHub issue closure and reopen history
<!-- devlog-id: 2026-11-22-issue-2684-github-events -->

### Result

- forge-cli issue events exposes the GitHub timeline with actor, time and
  supplied closer references.

### Why / context

- GitHub timeline cursors preserve matching events beyond the first page and
  allow the CLI to fail without returning a partial audit history.

### Evidence

- The CLI fixture exercised 102 events, and a live CLI read returned a GitHub
  closure.

### Links

- <https://github.com/sympoies/nils-cli/issues/2684>
