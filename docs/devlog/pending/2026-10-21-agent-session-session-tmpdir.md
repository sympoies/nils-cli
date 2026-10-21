## 2026-10-21 - agent-session: per-session TMPDIR under AGENT_HOME/tmp with reclaim
<!-- devlog-id: 2026-10-21-agent-session-session-tmpdir -->

### Result

- Managed tmux sessions now launch with `TMPDIR=${AGENT_HOME}/tmp/<session-id>`
  (mode 0700); TMUX_TMPDIR and the launching process environment are unchanged.
- The directory is reclaimed after a grace period (default 24 h,
  AGENT_SESSION_TMP_GRACE_SECS) once the runtime exits, on relaunch, and by the
  new agent-session tmp sweep command, which also removes orphans and reports
  what it removed. A running or unprovable session is never swept.
- A session can keep the system TMPDIR with the
  AGENT_RUNTIME_SESSION_TMPDIR=system launch switch.

### Why / context

- Harness scratchpads and mktemp output landed in the system /tmp and nothing
  reclaimed them (agent write-location contract, tmp class).

### Evidence

- Unit tests cover create, inject, opt-out, relaunch reclaim, live-session
  refusal, grace-gated reclaim and orphan sweep; integration tests cover the
  tmp sweep command and the start/resume tmux argv.
- A headless Claude Code run and a Codex run with TMPDIR set both put their
  scratch under it, and Codex mktemp resolves there in the default
  workspace-write sandbox.

### Links

- <https://github.com/sympoies/nils-cli/issues/2727>
- <https://github.com/sympoies/agent-runtime-kit/issues/302>

### Follow-ups

- Run the sweep periodically from the serve daemon; today it runs at launch and
  on demand.
