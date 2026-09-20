# Codex Capacity Auto-Continue Handoff

- Status: accepted for implementation
- Date: 2026-09-20
- Source: user request, Agent Console PR #493, current `agent-session`
  contracts, and the locally installed Codex 0.153.4 app-server schema
- Intended outcome: let an opted-in managed Codex session recover from a
  structured model-capacity interruption without terminal-text automation or
  an OpenAI Codex change

## Problem

Codex can finish a turn with the visible message `Selected model is at
capacity. Please try a different model.`. The work stops until another prompt
is submitted. Agent Console must not infer this condition from terminal output
or type text and Enter into a pane, because untrusted output can reproduce the
same text and the pane may instead contain an approval or question dialog.

## Existing trusted boundary

`agent-session` already owns the required primitives:

1. Managed Codex sessions use the daemon-owned app-server v2 control channel.
2. The failure reducer correlates the bound thread and failed turn and accepts
   only a terminal non-retrying `codexErrorInfo: serverOverloaded` failure.
3. The activity projection records `provider_capacity` on the exact failed
   turn.
4. Durable auto-resume already provides opt-in state, runtime and activity
   fences, manual-input cancellation, a durable pre-submit claim, and
   acknowledged `turn/start` submission.
5. The installed Codex 0.153.4 JSON schema still declares
   `serverOverloaded`; no upstream change is required.

## Decisions

1. Extend daemon-owned auto-resume rather than adding Agent Console terminal
   matching or browser-side key injection.
2. Admit capacity recovery only for an already-enabled auto-resume session on
   a supported managed Codex app-server runtime.
3. Use this fixed, server-owned prompt:
   `The selected model was at capacity, interrupting the previous turn. Please continue from where you stopped.`
4. Submit through app-server `turn/start`, never through the terminal pane.
5. Preserve exact runtime incarnation, failed-turn projection, activity
   revision, no-pending-attention, and manual-input/account-mutation fences.
6. Use the existing bounded retry delays (30, 60, 120, 300, and 600 seconds)
   across one capacity-recovery chain. A repeated structured capacity failure
   advances that chain; after five attempts it becomes a visible terminal
   auto-resume failure and never loops indefinitely.
7. Persist the retry claim before submission. An indeterminate submission is
   terminal and is not replayed after restart.
8. Keep terminal prose, prompt text, output, raw thread IDs, and raw turn IDs
   out of durable state and public projections.

## Acceptance criteria

- A matched `serverOverloaded` failure arms recovery exactly once when
  auto-resume is enabled.
- Raw TUI text, unknown error kinds, retrying errors, wrong thread/turn,
  reordered partial events, raw Codex sessions, and disabled auto-resume never
  arm recovery.
- The retry sends the exact fixed English prompt through the bound control
  connection only after all state fences still match.
- Manual input, account mutation, runtime replacement, activity revision
  change, pending attention, cancellation, and unhealthy protocol state win
  before submission.
- Repeated capacity failures back off and stop after the bounded chain; a
  successful acknowledged new turn clears the capacity chain.
- Restart discovers scheduled work but never replays a claimed or unknown
  submission outcome.
- Existing quota-reset and next-account recovery behavior remains unchanged.
- Main Agent supervision waits for the daemon-owned recovery and never sends a
  competing raw retry.
- Focused Rust tests and the repository local-fast gate pass.

## Read first

- <https://github.com/serenvia/agent-console/pull/493>
- `crates/agent-session/docs/provider-turn-signal-evidence.md`
- `crates/agent-session/docs/turn-state-contract.md`
- `crates/agent-session/docs/specs/serve-api-v1.md`
- `crates/agent-session/src/codex_app_server.rs`
- `crates/agent-session/src/activity.rs`
- `crates/agent-session/src/auto_resume.rs`
- `crates/agent-session/src/main_agent.rs`

## Execution

- Recommended plan: docs/plans/2026-09-20-codex-capacity-auto-continue/codex-capacity-auto-continue-plan.md
- Recommended execution state: docs/plans/2026-09-20-codex-capacity-auto-continue/codex-capacity-auto-continue-execution-state.md
