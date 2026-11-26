## 2026-11-26 - Stop a retained source before a cross-host resume move
<!-- devlog-id: 2026-11-26-issue-4058-resume-move-stop -->

### Result

- Added an exact-incarnation graceful stop for idle Codex and Claude sources.

### Why / context

- Move export needs a stopped source without deleting its record or history.

### Evidence

- Synthetic stop and signed export cover unchanged record and history bytes,
  concurrent stop, and stale selectors.

### Links

- <https://github.com/sympoies/nils-cli/issues/4058>
