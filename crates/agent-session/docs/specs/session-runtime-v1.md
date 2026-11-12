# Session runtime v1

Direction record for [#3585](https://github.com/sympoies/nils-cli/issues/3585),
the structured session runtime spike. This document states the direction and
the requirements later work follows, including
[#3582](https://github.com/sympoies/nils-cli/issues/3582),
[#3583](https://github.com/sympoies/nils-cli/issues/3583) and
[#3584](https://github.com/sympoies/nils-cli/issues/3584). It precedes
implementation: it does not claim that any released daemon behaves this way.
The existing turn signals stay owned by
[Turn state contract](../turn-state-contract.md), and terminal input and attach
stay owned by [Serve terminal transport v1](serve-terminal-v1.md).

## Evidence basis

Provider facts in this document come from the #3585 spike, read and measured
on 2026-11-12 against Claude Code 2.1.317, the Claude Agent SDK 0.3.322 and
`codex-cli` 0.162.1. They are pinned to those versions; the provider signals
are not a stable contract. Statements are labeled where it matters:

- **[F]** fact: measured in the spike or quoted from the provider's documents.
- **[I]** inference: reasoned from facts, including from the absence of a
  documented feature.
- **[N]** not measured.

Unlabeled sentences are requirements or decisions, not provider claims.

## Direction

The target is hybrid. Wherever a provider offers an official structured source,
`agent-session` reads it instead of inferring state from the screen, a pane
quiet window or a hook-only heuristic. A human can still take over any
session in the real provider terminal UI.

| Provider | Session runtime | Structured sources | Takeover and live view |
| --- | --- | --- | --- |
| Codex | `codex app-server` owns the thread | JSON-RPC turn, interrupt, steer and approval requests | Codex TUI attached to the same app-server |
| Claude | Claude TUI in tmux (option C) | `claude agents --json` state, hook events, transcript | The tmux pane itself |

The Codex row is **[F]** from the generated app-server protocol schema and one
real-turn probe (n=1: start, turn, restart, resume, fork). Attaching the Codex
TUI with `--remote` is **[F]** from the CLI help only; the spike did **[N]**
exercise it.

Maintainer decision, 2026-11-12, recorded on #3585: option C for Claude.
Options evaluated in the spike (comment on #3585):

- **Full Claude Agent SDK runtime (option A): not adopted** as a default.
  It gives structured turn, interrupt and approval signals **[F]**, but Claude
  offers no supported way to attach a live terminal UI to an SDK or
  stream-json session **[I]**: none was found in the provider documents and
  package read in the spike, two long documentation pages of which were read
  only in part.
- **Mode-switch hybrid (option B): possible later opt-in, not scheduled.** A
  switch between an SDK process and the terminal UI costs a few seconds and
  loses runtime state such as the permission mode. It could return as an
  opt-in runtime for unattended sessions.
- **TUI plus structured sources (option C): decided for Claude.** It keeps the
  live view and replaces heuristics with official signals.

## Requirements

Each requirement uses RFC 2119 terms.

1. **Takeover.** A session MUST remain takeoverable in the real provider
   terminal UI. A runtime that cannot show a live terminal view MUST be an
   explicit per-session opt-in and MUST NOT be the default for a session that
   needs a live view.
2. **Structured first.** State, approval and completion detection MUST use an
   official provider signal where one exists. A screen or timing heuristic MAY
   remain only where no official signal exists, and the provider evidence
   document MUST name it. When a structured source replaces a heuristic, the
   same change deletes the heuristic; the two are not kept side by side.
3. **Single writer.** At most one writer MAY drive a provider session id at any
   time. A writer is a provider process that owns the session's live
   conversation state and can append to it: a terminal UI process, or a
   stream-json or SDK host process. Clients of a process that already is the
   writer are views of it, not writers: a Codex TUI attached to the app-server
   and control-channel prompts to the same thread are clients of the
   app-server, which is the single writer and serializes their turns.
4. **Provider environment.** A process spawned by a runtime MUST receive an
   explicitly built environment. It MUST NOT inherit API-key style credentials
   or endpoint overrides unless the session is configured for API billing, and
   the billing mode MUST be recorded. Provider precedence puts such variables
   above a subscription login, so an inherited key silently changes billing.
5. **Credentials and terms.** A runtime MUST use only logins the operator owns,
   through the provider's own login store. It MUST NOT collect, copy or
   intermediate subscription credentials, and MUST NOT offer subscription
   login or limits to other users.
6. **Switch state.** State that does not survive a writer switch MUST be
   re-declared by the new writer. A writer MUST NOT release a session while an
   approval is pending without surfacing it, because pending approvals are
   cancelled when the writer's input closes.

## Single-writer rule

The spike measured, once (n=1, Claude Code 2.1.317, **[F]**), that Claude
provides no session lock: a stream-json resume succeeded while a terminal UI
held the same session open, the terminal UI did not see the stream-json turn
(its own answer denied it; the stream-json side was not asked), and the later
stream-json resume followed the terminal UI's branch, so the stream-json turn
left the effective context. One run does not bound how often or how silently
this happens, so the rule below is a requirement, not a tuned mitigation. The
runtime therefore owns it:

| Part | Requirement |
| --- | --- |
| Lease | One ownership lease per provider session id, in a durable per-session record owned by `agent-session` (its exact location is an open question). It names the writer kind and process. |
| Acquire | Before any resume or attach, check the lease and the provider's live-process listing for the session id. Refuse when another writer holds it. |
| Handoff | An explicit command: stop the current writer, verify its process has exited, release the lease, then start the next writer. |
| Notification | The lease record notes the handoff. The outgoing writer learns by its own exit; the incoming writer by acquiring the lease. |
| Codex | The app-server is the writer for its running threads. The TUI and control-channel clients attach to that process as views; no separate process resumes a copy of a running thread. |

Stale leases after a crash and the exact lease location are open questions
below.

## Provider signal map

This map is current evidence for the pinned versions above. It is not a
contract until implemented and tested. Basis per cell:

- Claude turn state, approval request and approval answer: **[F]**, measured
  on one tmux session per signal (n=1; the hook decision allow and deny once
  each).
- Claude interrupt by keystroke (observed through `agents --json`, n=1) and
  Claude input, images and model switching by keystrokes and file references:
  the interrupt is **[F]**; that no structured channel exists for the rest is
  **[I]**.
- Codex cells: **[F]** from the schema; turn lifecycle, resume and fork from
  the one probe; interrupt, steer and approval answers **[N]** exercised.

| Need | Claude (TUI in tmux) | Codex (app-server) |
| --- | --- | --- |
| Turn state | `claude agents --json` `status` (`idle`, `busy`, `waiting`) and `waitingFor`; `Stop` hook, completing after a one-second quiet window | `turn/started`, `turn/completed` notifications |
| Approval request | `PermissionRequest` hook, bound to its `PreToolUse` call by turn, tool name and input digest; cleared by that call's result hook | Server requests with JSON-RPC ids |
| Approval answer | Hook decision, while the terminal dialog stays visible | JSON-RPC response |
| Interrupt | Keystroke, observed through `agents --json` | `turn/interrupt` |
| Input and images | Keystrokes and file references | `turn/start` input items |
| Resume and fork | `claude --resume`, `--fork-session` | `thread/resume`, `thread/fork` (a thread needs one turn first) |

Hook signals as implemented by
[#3591](https://github.com/sympoies/nils-cli/issues/3591), under the
[Turn state contract](../turn-state-contract.md):

- **Stop window.** The one-second quiet window after a qualifying `Stop` stays.
  Claude runs matching `Stop` hooks in parallel and any can continue the turn,
  and the `Stop` payload cannot show that none will **[F]** (official
  contract). `claude agents --json` idle (#3590) is the structured source that
  removes the window, and the screen rules follow in #3592 after both land.
  Immediate completion remains limited to `StopFailure` and `idle_prompt`.
- **Approval correlation.** `PermissionRequest` carries `tool_name` and the
  call's `tool_input` but no call id **[F]**. It is bound to the open
  `PreToolUse` call of the same turn with the same tool name and input digest
  when exactly one matches, and then cleared by that call's `PostToolUse`,
  `PostToolUseFailure` or `PermissionDenied`. Otherwise it keeps the
  conservative latch. The tool name is recorded; the input never is.
- **No hook on denial.** A manual denial and an interrupt emit no hook **[F]**
  (n=1 each), so a recorded wait ends only at the next prompt, a same-turn
  `Stop` or a runtime boundary until a structured state source observes it.
- **Not answered here.** The hook only records; the terminal dialog stays the
  answerer. Answering through the hook is staging step 3.
- **Without hooks.** A session whose managed hooks are absent or incomplete
  keeps the existing evidence: approvals stay `conservative`, completion
  waits for `idle_prompt`, and `activity doctor` reports the missing events.

## Staging

Order only; each step is its own issue.

1. Claude state reader over `claude agents --json`, replacing the post-Stop
   quiet window and two-sample interrupt checks in the same change.
2. Single-writer lease, handoff command and pre-attach liveness check.
3. Approval answers through the `PermissionRequest` hook with id correlation.
4. Codex slices #3582, #3583 and #3584 under this document.

Not scheduled: an opt-in unattended stream-json runtime (option B). It would
need its own maintainer decision and would reuse the single-writer rule.

## Open questions

- Agent view is a provider research preview. What provider version window does
  the state reader support, and what is the failure behavior outside it?
- A manual approval denial and an interrupt emit no Claude hook. Which
  structured source ends the recorded wait sooner than the next prompt: the
  `claude agents --json` state (#3590), or the transcript?
- Each `claude agents --json` call took about a third of a second. What polling
  cadence, or event source, keeps cost acceptable across many sessions?
- Can `claude --bg` sessions with `attach` be a better live-view base, given
  that the provider CLI has no command to send input to them?
- Where does the lease live, and how is a stale lease detected after a crash?
- For Codex, how do a separate TUI process and the app-server behave on the
  same stopped thread, and how is remote attach authenticated?
- Should an unattended runtime depend on the provider SDK package or speak the
  provider stdio protocol directly? No stability promise was found for the raw
  wire format.
- Subscription terms describe "ordinary, individual usage". Revisit this
  document if the provider terms or credit rules change.
