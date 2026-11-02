## 2026-11-02 - Ignore subagent auth failure for the main session
<!-- devlog-id: 2026-11-02-issue-3218-subagent-auth-failure -->

### Result

- A Claude StopFailure carrying agent_id (fired inside a subagent) no longer
  opens an auth incident or quarantines the main session; a main-thread failure
  still quarantines.
- The auth-quarantined refusal exposes a fixed reason and exit list with no
  credential or path data.

### Why / context

- A background subagent's revoked-token error at credential refresh quarantined
  a healthy main session and refused all input.

### Evidence

- Two synthetic-payload tests failed before the change (incident created; no
  typed reason) and pass after it. The local-fast gate passes except the known
  startup_failure case tracked by #3182.

### Links

- <https://github.com/sympoies/nils-cli/issues/3218>
