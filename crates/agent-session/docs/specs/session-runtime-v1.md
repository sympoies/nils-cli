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

## Direction

The target is hybrid. Wherever a provider offers an official structured source,
`agent-session` reads it instead of inferring state from the screen, a pane
quiet window or a hook-only heuristic. A human can still take over any
session in the real provider terminal UI.

| Provider | Session runtime | Structured sources | Takeover and live view |
| --- | --- | --- | --- |
| Codex | `codex app-server` owns the thread | JSON-RPC turn, interrupt, steer and approval requests | Codex TUI attached to the same app-server |
| Claude | Claude TUI in tmux (option C) | `claude agents --json` state, hook events, transcript | The tmux pane itself |

Decisions recorded from the spike (comment on #3585):

- **Full Claude Agent SDK runtime (option A): not adopted** as a default. It
  gives structured turn, interrupt and approval signals, but Claude offers no
  supported way to attach a live terminal UI to an SDK or stream-json session.
- **Mode-switch hybrid (option B): deferred.** A switch between an SDK process
  and the terminal UI costs a few seconds and loses runtime state such as the
  permission mode. It may return later as an opt-in runtime for unattended
  sessions.
- **TUI plus structured sources (option C): chosen for Claude.** It keeps the
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
   time. A writer is any process that can submit input to or resume the
   session: a terminal UI, a stream-json or SDK host, an app-server client.
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

The spike measured that Claude provides no session lock: a stream-json resume
succeeded while a terminal UI held the same session open, neither side saw the
other's turn, and the later resume followed one branch so the other turn left
the effective context. The runtime therefore owns the rule:

| Part | Requirement |
| --- | --- |
| Lease | One ownership lease per provider session id in the session registry. It names the writer kind and process. |
| Acquire | Before any resume or attach, check the lease and the provider's live-process listing for the session id. Refuse when another writer holds it. |
| Handoff | An explicit command: stop the current writer, verify its process has exited, release the lease, then start the next writer. |
| Notification | The registry records the handoff. The outgoing writer learns by its own exit; the incoming writer by acquiring the lease. |
| Codex | The app-server owns running threads. Clients attach to that process; they do not resume a separate copy of a running thread. |

Stale leases after a crash and the exact registry location are open questions
below.

## Provider signal map

This map is current evidence for the pinned provider versions in the spike. It
is not a contract until implemented and tested.

| Need | Claude (TUI in tmux) | Codex (app-server) |
| --- | --- | --- |
| Turn state | `claude agents --json` `status` (`idle`, `busy`, `waiting`) and `waitingFor`; `Stop` hook | `turn/started`, `turn/completed` notifications |
| Approval request | `PermissionRequest` hook; `PreToolUse` carries the tool call id | Server requests with JSON-RPC ids |
| Approval answer | Hook decision, while the terminal dialog stays visible | JSON-RPC response |
| Interrupt | Keystroke, observed through `agents --json` | `turn/interrupt` |
| Input and images | Keystrokes and file references | `turn/start` input items |
| Resume and fork | `claude --resume`, `--fork-session` | `thread/resume`, `thread/fork` (a thread needs one turn first) |

## Staging

Order only; each step is its own issue.

1. Claude state reader over `claude agents --json`, replacing the post-Stop
   quiet window and two-sample interrupt checks in the same change.
2. Single-writer lease, handoff command and pre-attach liveness check.
3. Approval answers through the `PermissionRequest` hook with id correlation.
4. Codex slices #3582, #3583 and #3584 under this document.
5. Optional unattended stream-json runtime, only after a maintainer decision.

## Open questions

- Agent view is a provider research preview. What provider version window does
  the state reader support, and what is the failure behavior outside it?
- The `PermissionRequest` hook payload carries no tool call id. What
  correlation rule with `PreToolUse` is sufficient and testable?
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
