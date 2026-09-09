# Codex TUI Busy Turn Start — Implementation Source

## Decision

Repair the Codex app-server proxy in `nils-agent-session`. A valid TUI
`turn/start` from a managed-account session must be authorized by the same
exact-runtime and account-mutation fences used by structured input, while local
rejections expose only a bounded privacy-safe reason code.

## Evidence

- A resumed production session rejected repeated TUI submissions with local
  JSON-RPC code `-32001` while its authoritative activity state was waiting.
- The same prompt submitted once through the exact-incarnation `prompt/v2`
  route was acknowledged and the session returned to working.
- A controlled freebox session with an init prompt completed its first turn,
  then rejected its next ordinary TUI turn with the same local error.
- A controlled freebox session without an init prompt rejected its first
  ordinary TUI turn with the same local error.
- Both canaries were bound to the selected account and their app-server runtime
  and thread attachment were ready. No provider turn was created for either
  rejected prompt.
- The rejection was introduced in the local `turn/start` authorization path;
  quota, provider availability, credentials, and global account switching are
  therefore not the root cause.

## Required behavior

1. A valid first or subsequent TUI `turn/start` for the currently bound account
   is forwarded exactly once.
2. A queued account change, replacement runtime, malformed request, or second
   in-flight start remains rejected before provider forwarding.
3. The stable TUI JSON-RPC error shape remains compatible.
4. Diagnostics expose one closed-vocabulary rejection reason and no
   user-controlled or identifying values.
5. Existing structured prompt, reconnect, account switching, and tmux pane
   ownership contracts remain unchanged.

## Deployment boundary

After reviewed merge, publish and deploy the next stable `nils-cli`, restart
the Agent Console serve daemon only after confirming `KillMode=process`, retain
all panes, then repeat both controlled freebox session shapes.

## Execution

- Recommended plan: `docs/plans/2026-09-09-codex-tui-busy-turn-start/2026-09-09-codex-tui-busy-turn-start-plan.md`
- Recommended execution state: `docs/plans/2026-09-09-codex-tui-busy-turn-start/2026-09-09-codex-tui-busy-turn-start-execution-state.md`
- Status: execute through live acceptance and strict provider closeout.
