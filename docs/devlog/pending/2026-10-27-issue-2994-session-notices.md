## 2026-10-27 - Guarded terminal delivery for session notices
<!-- devlog-id: 2026-10-27-issue-2994-session-notices -->

### Result

- Agent-session notices can use exact composer verification for terminal
  runtimes and queue while a target is busy, with one guarded delivery.

### Why / context

- Joined logical rows and cursor-position checks distinguish terminal soft
  wraps from hard edits. Claude terminal refuses ASCII spaces at row endings
  before key claim or paste, including LF, CRLF, and the final row at EOF; the
  codec and native Codex delivery keep exact body bytes.

### Evidence

- The focused Claude notice cases passed (6/6), the complete guarded-send suite
  passed (116/116, single-threaded), and docs-only checks, formatting, diff
  checks, and strict file-size audit passed. A two-thread full-suite run had
  one mixed-draft observation timeout; the isolated test and single-thread full
  suite passed.

### Links

- <https://github.com/sympoies/nils-cli/pull/2997>
- <https://github.com/sympoies/nils-cli/issues/2994>

### Follow-ups

- Issue #3002 tracks the exact logical-input proof needed to lift the Claude
  terminal restriction; issue #3003 tracks producer-side checks for future
  internal notice writers.
