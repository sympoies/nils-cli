## 2026-10-27 - Guarded terminal delivery for session notices
<!-- devlog-id: 2026-10-27-issue-2994-session-notices -->

### Result

- Agent-session notices can use exact composer verification for terminal
  runtimes and queue while a target is busy, with one guarded delivery.

### Why / context

- Joined logical rows and cursor-position checks distinguish terminal soft
  wraps from hard edits. Claude terminal refuses ASCII spaces at row endings
  before key claim or paste, including LF, CRLF, lone CR, and the final row at
  EOF; the codec and native Codex delivery keep exact body bytes.

### Evidence

- The focused Claude notice cases passed (7/7), Clippy passed with warnings
  denied, and formatting, diff, and strict file-size checks passed. The latest
  117-test guarded-send run had two existing live-composer observation
  failures; both tests passed individually. Earlier, the 116-test suite passed
  single-threaded before the lone-CR regression was added.

### Links

- <https://github.com/sympoies/nils-cli/pull/2997>
- <https://github.com/sympoies/nils-cli/issues/2994>

### Follow-ups

- Issue #3002 tracks the exact logical-input proof needed to lift the Claude
  terminal restriction; issue #3003 tracks producer-side checks for future
  internal notice writers.
