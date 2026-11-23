## 2026-11-24 - Avoid self-matching in Claude rate-limit diagnostics
<!-- devlog-id: 2026-11-24-claude-diag-active-target -->

### Result

- Resolve the active login name against saved profile names when the config and
  profile directories coincide.

### Why / context

- Scanning all JSON files admitted the active credentials file as its own
  saved-profile match and recursed before the usage request.

### Evidence

- The loopback integration regression reproduced a stack overflow on the
  baseline and passed after the fix. All 27 rate-limit diagnostic integration
  cases passed.
