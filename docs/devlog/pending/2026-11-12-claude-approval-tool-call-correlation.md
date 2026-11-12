## 2026-11-12 - Claude approval waits correlate to their tool call
<!-- devlog-id: 2026-11-12-claude-approval-tool-call-correlation -->

### Result

- A Claude PermissionRequest is now bound to its PreToolUse call by turn, tool
  name and input digest, recorded with the tool name, and cleared by that
  call's own PostToolUse or PostToolUseFailure instead of waiting for the next
  Stop.

- The one-second post-Stop quiet window stays and is now documented: parallel
  Stop hooks can continue a turn and the Stop payload cannot rule that out; the
  claude agents --json idle state (#3590) is what removes it.

- Release note: install agent-session and agent-hook together in one install
  step on each host (agent-session first if the installer orders them). An
  older agent-session rejects the new tool_call event key, and the resulting
  session-activity-failed blocks mutating Claude tool calls under enforce
  coordination. There is no compatibility guard in the hook.

### Why / context

- PermissionRequest has no tool_use_id and no resolution hook, so the old latch
  cleared only at a same-turn Stop. A hook-only reading cannot see a manual
  denial or an interrupt.

### Evidence

- Probe on Claude Code 2.1.317 in tmux (n=1 approved, n=1 denied):
  PermissionRequest repeats the PreToolUse tool_input without an id; the
  approved call's result hook followed with the call id; a manual denial
  emitted no hook.
- Failing-first agent-session reducer tests, agent-hook wiring tests, and a
  mutation check showing the ingest test fails without the tool-call duplicate
  key.

### Links

- sympoies/nils-cli#3591
- sympoies/nils-cli#3585

### Follow-ups

- Observe manual denial and interrupt through the claude agents --json state
  (#3590); remove the screen rules (#3592) after both land.
