## 2026-11-16 - Schedule safe per-session temporary-directory retention
<!-- devlog-id: 2026-11-16-issue-3749-session-tmp-retention -->

### Result

- Every serve role runs session-tmp retention at startup and hourly; foreign
  roots and uncertain candidates are preserved, with count-only reporting.

### Why / context

- The previous owner only swept on launch or explicit command, so expiry alone
  never triggered reclamation. Session locks now cover deletion and missing
  records are distinguished from unreadable records.

### Evidence

- Synthetic regression tests failed before production edits for missing daemon
  trigger, foreign-root deletion and value-bearing reports; focused CLI sweep
  tests pass after repair.

### Links

- <https://github.com/sympoies/nils-cli/issues/3749>
