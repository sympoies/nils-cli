# Serve prompt submission v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## Authenticated writes and prompt submission

- `POST /sessions` (create), `PATCH /sessions/{id}` (title update), `POST /sessions/{id}/send`,
  `POST /sessions/{id}/prompt`,
  `POST /sessions/{id}/resume`,
  `PUT /sessions/{id}/account`,
  `PUT /sessions/{id}/auto-resume`, `DELETE /sessions/{id}/auto-resume`,
  `POST /sessions/{id}/attachments?filename=...`,
  `DELETE /sessions/{id}` — writes, require a bearer token.
- `POST /sessions/{id}/prompt` submits exact prompt text through a supported provider control plane. The
  compatibility route accepts `{ "text": "...", "expected_session_incarnation": "launch-id" }`; the
  incarnation is optional for older clients, and a new daemon validates it against the authoritative
  runtime under the session-record lock before provider dispatch. Clients that require a cross-version
  fence use `POST /sessions/{id}/prompt/v2`, which requires both fields, rejects unknown fields, and is
  absent from older daemons so they fail before provider dispatch. A replacement returns HTTP 409
  `session-incarnation-conflict` without submitting.

  Success returns `submitted: true` plus the locked `session_incarnation`, while the provider turn id
  remains private. These mutations never send multiline text through terminal keys; unsupported or
  not-yet-ready sessions fail closed.

  If provider acknowledgement is lost while the recorded exact tmux runtime is confirmed stopped, the
  route treats the prompt as submitted. It probes the runtime identity recorded for the locked session,
  not the reusable tmux session name. Missing identity, probe errors, and a runtime that remains live do
  not confirm submission and retain the unknown outcome.

  On a Claude (pane-delivered) runtime, success means Claude's `UserPromptSubmit` hook started a new turn within the
  acknowledgement wait, a newly appended provider queue record matches the complete prompt, or the complete
  whitespace-free prompt gains a pane queue sighting relative to a readable pre-submit snapshot.
  Acknowledgement snapshots are captured under the same session-record lock as their own paste, so concurrent
  requests cannot reuse an earlier lock holder's evidence. A pre-existing
  sibling or identical entry alone cannot acknowledge the new attempt; a shared prefix never suffices.
  Missing or ambiguous baseline pane evidence cannot prove a queue transition, but a new matching provider
  queue record can still confirm it. A tool-call turn can absorb the queue at its next tool call, so any proven
  sighting within the wait suffices. The queued case adds `queued: true` and the client MUST NOT resend it.
  A new turn omits `queued`. Without `require_idle`, when neither is observed the route answers
  HTTP 500 `structured-prompt-outcome-unknown`. Guarded outcomes follow the HTTP 409 rules below.

  Direct structured replies through either route inspect the composer under the
  submission's session-record lock before terminal paste or native dispatch.
  An observed owner draft returns HTTP 409 `composer-not-empty`, preserving the
  draft and sending nothing. An unreadable or ambiguous composer returns HTTP
  409 `composer-unobservable`, also before dispatch. These refusals preserve
  account/input control state and automatic recovery settings. The check uses
  the existing exact composer observation limits in [Send v1](send-v1.md).
  It is non-atomic against human keystrokes or provider redraws after the check.

  A Claude queued-reply hint is empty only when its sole input row is the
  native dim-styled hint and contains no owner text. A second direct reply is
  then admitted. Unstyled literal text matching the hint, or owner input beside
  it, remains a draft and returns `composer-not-empty`.

  Codex numbered dialogs do not prove an empty composer and return
  `composer-unobservable`; no keys are sent to select an option. Once the dialog
  is resolved, retry only after observing an empty composer, using a fresh
  request ID after a keyed refusal. Unreadable panes still refuse, and refusals
  leave recovery armed. An empty composer behind a working Codex turn remains
  admitted, subject to the existing provider controls.

  An explicit owner clear followed by another submission is the draft recovery path;
  the reply operation never clears a draft or sends Ctrl+C. Direct replies
  retain their existing provider acknowledgement and queue confirmation rules
  when the composer is observably empty. Initial creation prompts and internal
  coordination notifications retain their separately owned admission policy.

  The v2 route additionally accepts optional `request_id` and `require_idle`
  fields. Omitting both preserves the existing response and delivery behavior.
  `request_id` contains 1–128 ASCII letters, digits, dots, underscores or
  hyphens; invalid IDs return `400 invalid-request-id`. `require_idle` is a
  boolean, default false. Malformed JSON/types or other unknown fields return
  `400 invalid-json-body` before dispatch.

  A keyed attempt is scoped to the canonical session ID, exact expected
  incarnation and request ID. The daemon durably claims that key before
  dispatch. Repeating the same text and idle flag returns or safely confirms the
  recorded outcome without sending again; different text or idle flag returns
  `409 request-id-conflict`, preserving the original receipt. An in-flight,
  interrupted or incomplete claim returns `unknown`, never authorizing another
  dispatch. This is at-most-once dispatch, not a guarantee of delivery or turn
  completion.

  Every keyed outcome contains this receipt (identifiers are illustrative):

  ```json
  {
    "request_id": "attempt-1",
    "session_id": "session-id",
    "session_incarnation": "launch-id",
    "outcome": "submitted",
    "code": null
  }
  ```

  `outcome` is `submitted`, `refused` or `unknown`. `code` is null for
  submitted, the stable refusal code for refused, or
  `structured-prompt-outcome-unknown` for unknown. Successful keyed POSTs retain
  `data.machine`, `data.submitted: true`, `data.session_incarnation` and the
  optional `data.queued: true`, adding `data.receipt`. Refused/unknown POSTs use
  `ok: false` and place the receipt in `error.details.receipt`. Unknown returns
  HTTP 500 with `error.code: structured-prompt-outcome-unknown`, except for
  guarded dispatch failures described below, which return HTTP 409.

  For a Codex request, history may include a `prompt_request` object on the
  matching user message. Its `request_id`, `session_id` and
  `session_incarnation` equal the keyed receipt fields. The daemon joins these
  values using the provider's accepted turn identifier; the provider identifier
  itself is not exposed. The association remains stable across receipt lookup
  and history windows. Messages without supported evidence omit the field;
  Claude requests remain unbound pending provider evidence.

  HTTP status alone does not establish non-delivery: clients MUST read `receipt.outcome`
  for keyed attempts and `error.code` for unkeyed attempts. Repeated POSTs
  return the retained status and outcome; an unfinished claim may later resolve
  through lookup when the original request completes.

  A keyed Codex idle-only attempt may retain a private, request-correlated native
  turn acknowledgment when input history is not yet observable. Lookup or a
  matching repeated POST can then make one bounded, read-only confirmation of
  that exact turn and its single matching user input. The runtime, provider
  thread and account binding must still match the retained attempt before and
  after observation. A successful confirmation durably changes that receipt to
  `submitted`; later lookup and POST recover the same outcome without another
  input dispatch. The proof survives daemon restart and is not returned by the
  API. This does not add history waiting to ordinary immediate native input.

  Unknown receipts can include a value-free `confirmation_diagnostic`:
  `native_input_unconfirmed` means a native turn acknowledgment was retained but
  exact input confirmation remains unavailable; `acceptance_unobservable` means
  no recoverable acceptance evidence is available. These values never authorize
  replay. Older receipts may omit this field. Missing acknowledgments, unrelated
  or ambiguous input, changed identities and older attempts without retained
  proof remain unknown. Prompt text is not persisted in receipts or proofs.

  Lookup requires bearer authorization:
  `GET /sessions/{id}/prompt/v2/receipts/{request_id}?expected_session_incarnation=launch-id`.
  A found receipt of any outcome returns HTTP 200:

  ```json
  {
    "schema_version": "cli.agent-session.serve.v1",
    "ok": true,
    "data": {
      "machine": "machine-id",
      "receipt": {
        "request_id": "attempt-1",
        "session_id": "session-id",
        "session_incarnation": "launch-id",
        "outcome": "unknown",
        "code": "structured-prompt-outcome-unknown"
      }
    }
  }
  ```

  An absent key returns HTTP 404 with the same schema, `ok: false` and
  `error.code: prompt-receipt-not-found`. Missing/malformed lookup query returns
  `400 invalid-query`; invalid receipt identifiers return
  `400 invalid-receipt-key`. An older server's absent route does not return the
  typed `prompt-receipt-not-found` code. Neither an unsupported route nor
  not-found after an uncertain POST permits blind replay: the original POST
  may still be entering admission. Invalid receipt identifiers and
  authentication failures are rejected before creating receipts. Storage failure returns
  `prompt-receipt-storage-unavailable`; reconcile with the original key rather
  than assuming non-delivery.

  `require_idle: true` delegates to the CLI guarded send with exact runtime,
  provider idle, no pending provider ingress and an observably empty composer.
  Its existing record lock and pre-dispatch checks fence admission; Codex uses
  native guarded dispatch and Claude uses guarded pane delivery. A positive
  busy, pending-ingress or existing-draft observation before delivery returns
  HTTP 409 `busy`. Unobservable state fails closed with the CLI's stable guard
  code. These external provider/UI observations remain non-atomic, as in
  [guarded send](send-v1.md); a later failure after possible paste or native
  dispatch is `unknown`, even if no submission was confirmed.

  Owner-typed slash commands on the authenticated v2 conversation route always
  use the existing guarded live TUI composer, including when `require_idle` is
  absent or false. They never enter an app-server model turn. The route pins
  the runtime and provider conversation, requires idle activity and an empty
  composer, and retains account-input authority and submission receipts.
  Leading whitespace is removed before typing the slash command. Provider
  arguments remain subject to the existing single-line/control restrictions.

  The context-command denylist mirrors Salon's
  `packages/ui/src/terminalContextCommandGuard.ts`: Claude refuses `/clear`,
  `/new`, `/reset`, `/resume`, `/branch`, `/teleport`, `/tp`; Codex refuses
  `/new`, `/clear`, `/resume`, `/fork`. An argument does not bypass an exact
  command refusal; bare prefixes that can complete to a listed command also
  refuse. Changes to these lists must update both mirrors. Refusal returns
  `409 send-context-command-unsupported` with a terminal `refused` receipt and
  directs the owner to the managed session lifecycle.

  All other owner slash commands, including `/model`, `/effort`, `/status`,
  `/compact`, `/cost` and `/permissions`, go to the provider TUI. Only the exact
  authenticated owner `/login` command may enter an authentication-quarantined
  runtime as recovery; the quarantine remains until existing recovery observes
  success. Other input, generic send, raw input, notifications and peer/model
  callers retain their quarantine checks. No public owner-origin flag exists.
  A command dialog that the existing observer cannot recognize remains an
  `unknown` result; a terminal write alone never proves command execution.

  Guarded terminal submission retries one swallowed Enter only while its exact paste and all
  guards still hold. A stuck or unverified submission attempts the same bounded,
  identity-fenced cleanup as a post-paste guard failure. These uncertain guarded
  results return HTTP 409 `structured-prompt-outcome-unknown`, including without
  a request ID.

  Keyed receipts retain optional `composer_cleanup` with `cleared`
  (boolean) and `remaining_input` (`empty`, `pasted_text`, `different_text`, or
  `unobservable`). This reports residue without exposing composer contents;
  failed cleanup must not be treated as an empty composer. Cleanup does not
  prove non-dispatch: the receipt remains `unknown`, the key stays consumed,
  and clients reconcile that key instead of blindly resending. All guarded
  pre-delivery observation refusals return HTTP 409: positive busy observations
  use `busy`; identity, unsupported-observation, timeout and unobservable
  refusals preserve their stable guard code. Guarded dispatch also returns
  HTTP 409 `structured-prompt-outcome-unknown` if its blocking task fails or
  completes without a confirmed submission; these outcomes do not prove
  non-delivery and are never safe refusals.

  A `refused` receipt proves no provider dispatch/paste. Refusals, including
  `busy`, are terminal for that key. After a definite refusal and refreshed
  session state, a client may mint a new attempt ID for the same outbox item.
  Unknown attempts retain their original key and use lookup; clients MUST NOT
  rotate the key or fall back to unkeyed sends. Persist the attempt ID and
  expected incarnation before POST and verify receipt identifiers before
  acknowledging an outbox item.

  Private receipt claims and results use hashed filenames within session
  storage, synced before dispatch and atomically published after completion.
  They contain identifiers, a digest of text/idle policy and fixed outcome
  metadata, never prompt text or provider output. No automatic expiry or
  pruning is performed: receipts survive daemon restart and runtime
  replacement for the session lifetime. Explicit session deletion removes
  them; archive also ends the managed session and deletes its receipts. Lookup
  can read a historical incarnation even after replacement.

  For an already resumed Codex runtime whose terminal path reports local
  JSON-RPC `-32001` busy, the error alone does not prove rejection before
  provider acceptance: the same response can be emitted while an earlier
  request is already pending. Current proxies add `error.data.reason` from a
  closed vocabulary without user-controlled or identifying values while
  preserving the stable code and message. The current values are
  `turn_already_pending`, `account_mutation_forbidden`, `account_not_ready`,
  `turn_request_invalid`, `turn_gate_open_failed`, `turn_gate_busy`,
  `manual_marker_thread_mismatch`, `manual_marker_replaced`,
  `manual_marker_invalid`, `manual_ack_failed`, `runtime_identity_missing`,
  `manual_cancellation_busy`, `runtime_changed`, and
  `account_authority_unavailable`.

  Older proxies may omit `data.reason`, and
  clients MUST tolerate an absent or unknown reason while retaining the stable
  code/message fallback. Current proxies also add `error.data.retryable`
  (boolean), `error.data.next_action` (string), and
  `error.data.recovery` (an object with `outcome: "not_forwarded"`). These fields
  contain the following fixed values:

  | `data.reason` | `data.retryable` | `data.next_action` |
  | --- | --- | --- |
  | `turn_gate_busy` | `true` | `retry-request-after-state-change` |
  | `manual_cancellation_busy` with a persisted cause receipt | `true` | `retry-request` |
  | `account_authority_unavailable` with cause code `session-record-lock-timeout` and a persisted receipt | `true` | `retry-request` |
  | Other authority failures, missing receipts, `turn_already_pending`, and all other current reasons | `false` | `inspect-session-state` |

  `not_forwarded` describes only the frame rejected by this proxy response. It
  does not establish that an earlier request, including an earlier request with
  the same JSON-RPC ID, was never forwarded or accepted by the provider.
  `retryable: true` classifies this rejection as transient state contention;
  it does not authorize automatic replay. All next actions require the
  refresh/inspection and exact-incarnation recovery rules below. Clients MUST
  tolerate absent or unknown recovery fields from older proxies and preserve
  the stable code/message fallback and no-blind-replay behavior.

  Authority failures add a bounded `data.cause` with `operation`, `stage`, stable
  `code`, binding `binding_state` and `binding_revision`, and `receipt_written`. A private
  lifecycle receipt is persisted before cleanup, keyed to the exact session
  runtime and containing no account name, credential, provider output, or local
  path. Immediate cancellation retries only explicit record-lock `Busy`
  outcomes, at most three attempts. The timed cancellation path uses one
  six-second contention budget; other causes are returned without replay.

  A durably bound exact-runtime account remains valid in a detached tmux scope without inheriting the
  daemon's broker command; the broker is still mandatory for binding and account mutation. A client MUST
  first refresh `GET /sessions` and inspect provider-visible activity/output. While a turn is in
  progress, the continuation appears accepted, or non-delivery cannot be established, the client waits
  and MUST NOT submit it again. Only after establishing that no provider turn remains in progress and the
  continuation was not accepted may the client bind one explicit recovery request to the exact current
  `session_incarnation` and submit through `prompt/v2`.

  The incarnation fence prevents submission to a replacement runtime; unkeyed requests do not deduplicate
  two submissions within one incarnation. Clients MUST NOT automatically replay an arbitrary failed
  terminal send through this route because transport and busy failures can leave provider-delivery
  outcome unknown. Incarnation conflict and outcome-unknown handling retain the rules above.
