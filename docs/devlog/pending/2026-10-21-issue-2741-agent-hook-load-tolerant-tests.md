## 2026-10-21 - Make agent-hook finish-line and timeout tests load tolerant
<!-- devlog-id: 2026-10-21-issue-2741-agent-hook-load-tolerant-tests -->

### Result

- The finish-line tests wait on fixture markers, pid files and process exit
  (up to 30 s, returning as soon as the condition holds) instead of 2 s polls
  and fixed 250-400 ms sleeps.
- Negative checks no longer guess a window. Descendants record their pid and
  sleep far longer than any window, and the test requires the pid to be gone
  after the run returns; the user-manager delegation test asks the manager
  whether the delegated unit is active.
- An ordinary shell that must stay active while other requests are refused now
  waits for a release marker instead of sleeping 0.4 s.
- `AGENT_HOOK_HANDLER_TIMEOUT_MS` shortens the 5 s handler deadline and can
  never lengthen it; zero and unparsable values are ignored. Stalled-handler
  tests use a 2 s deadline against a 30 s stall with a 15 s ceiling, replacing
  a 5.8 s ceiling, and one of them asserts the shortened deadline in the
  incident record.

### Why / context

- Part of making full validation load tolerant so test threads can be raised
  (#2741, batch B7). A fixed window is a false pass under load; waiting on the
  thing that matters is both faster and stricter.

### Evidence

- The new incident-record assertion failed on the pre-change source
  (`deadline_ms` stayed 5000) and passes on this head.
- Touched finish_line, finish_line_acceptance and performance_contract tests
  plus the new unit tests: 103 of 103 green in 3 consecutive runs under 4
  nice-19 busy processes; the same set also passed pinned to one CPU with four
  busy loops, before and after the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/2741>

### Follow-ups

- The sleep-to-wait conversions could not be reproduced as failures on the
  build host before the change (the baseline set also passed pinned to one
  CPU with four busy loops), so only the deadline knob has a RED.
