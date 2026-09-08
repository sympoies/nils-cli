# Agent Session Runtime Recovery — Implementation Source

## Decision

Repair the failure in `nils-cli`'s `agent-session` runtime owner. A missing tmux
server must be treated as an authoritative empty live-session set during the
Codex account reconnect fence, while genuine tmux inspection failures remain
fail-closed. Also make the already supported incarnation-fenced structured
prompt route the documented recovery path when raw terminal submission reaches
the local Codex proxy but is rejected as transiently busy.

## Evidence

- [U1] The operator observed Agent Console showing its session runtime offline
  on both mobile and macOS while the edge/debug API continued receiving data.
- [F1] The user service repeatedly exited with
  `codex-account-reconnect-fence-unavailable` before binding its HTTP listener.
- [F2] Historical Codex session records with account bindings existed, but tmux
  had no server and therefore no live sessions.
- [F3] `fence_codex_controls_before_listen` requires a successful batched tmux
  snapshot whenever any historical candidate exists; `tmux_session_snapshots`
  currently maps every non-zero tmux exit to unavailable.
- [A1] Starting one disposable tmux session allowed the next service restart to
  complete. Removing that disposable session after startup did not destabilize
  the daemon, and the full Agent Console smoke suite passed.
- [F4] One resumed Codex session then rejected ordinary terminal submission with
  JSON-RPC code `-32001` and `agent-session state is busy; retry the request`.
- [A2] Repeating raw terminal input did not recover the session. Submitting the
  same continuation through `POST /sessions/{id}/prompt/v2`, fenced by the exact
  current session incarnation, was acknowledged and the session returned to
  working.
- [I1] The startup outage and the stuck input were separate local control-plane
  failures. The first needs a semantic empty-snapshot fix; the second needs a
  deterministic operator recovery path, not blind raw-input retries.

## Required behavior

1. The Codex reconnect fence starts successfully when tmux authoritatively
   reports that no server or sessions exist, even if historical bound Codex
   records remain on disk.
2. Other tmux execution, protocol, or parsing failures remain unavailable and
   keep the reconnect fence fail-closed.
3. Focused regression coverage distinguishes the empty-server case from a real
   inspection failure.
4. The serve API and operator runbook describe the exact, incarnation-fenced
   structured prompt recovery for a resumed Codex TUI whose raw submission is
   rejected as locally busy.
5. Recovery guidance warns against repeated blind raw-input retries and against
   bypassing the session incarnation fence.
6. Existing session records, account bindings, tmux panes, and API wire shapes
   remain unchanged.

## Deployment boundary

After reviewed merge, publish the next safe stable `nils-cli` release through
the private fixed-fleet release workflow. Restart the Agent Console serve daemon
only through its governed runtime procedure, preserving live tmux panes, then
run the full stack smoke suite and verify the deployed binary version.

## Execution

- Recommended plan: `docs/plans/2026-09-08-agent-session-runtime-recovery/agent-session-runtime-recovery-plan.md`
- Recommended execution state: `docs/plans/2026-09-08-agent-session-runtime-recovery/agent-session-runtime-recovery-execution-state.md`
- Status: execute through reviewed merge, stable release, fixed-fleet deployment, and live acceptance.
- Next-task source: Sprint 1, Task 1.1 in the recommended plan.
