# Owner prompt intent design

Status: proposed, design only. Refs [#3114](https://github.com/sympoies/nils-cli/issues/3114)
and [#3062](https://github.com/sympoies/nils-cli/issues/3062).
The [current prompt contract](serve-prompt-v1.md) remains implemented behavior.
Designated review, consumer-owner agreement and owner sign-off on the explicit
command action are required before opening an implementation issue.

## Decision

Require `intent: "text" | "command"` on every owner prompt submission. The
composer defaults to text and offers an explicit command action; callers must
not select command intent automatically from a slash prefix. Persist the choice
with the outbox item. Do not infer intent from command names, argument wording,
sentence shape or punctuation.

Typing `/compact` in the composer will require an explicit "Send as command"
action. This visible behavior change requires owner sign-off on the consumer
side, in addition to design review and consumer-owner agreement.

This is smaller than maintaining provider-specific argument grammars: arguments
and extensions belong to the provider, and the current first-token classifier
cannot validate them. Command intent delegates argument interpretation to the
provider after existing safety checks. For `/model explain the available
models`, only the caller's explicit choice determines intent. Neither a known
command inventory nor a provider-version probe is needed.

Explicit text intent cannot make a provider TUI treat leading slash input as
literal text. Claude delivery pastes into that TUI; guarded Codex native input
also has command restrictions. Refuse command-shaped text uniformly for both
providers instead of adding escaping, modifying exact text, or inventing another
transport. The caller can put ordinary text before the slash expression, or
explicitly select command intent. This is a typed input/intent conflict, never
an idle requirement or attempted command. Exact leading-slash literal text
support would need a separate provider transport design.

## Explicit-intent request form

The explicit-intent form uses the existing authenticated
`POST /sessions/{id}/prompt/v2` route:

```json
{
  "text": "Explain what /login does",
  "intent": "text",
  "expected_session_incarnation": "launch-id",
  "request_id": "attempt-1",
  "require_idle": false
}
```

`text`, `intent`, and `expected_session_incarnation` are required. `request_id`
remains optional; `require_idle` remains a boolean defaulting to false. Unknown
fields remain invalid. Null, non-string or unrecognized intent returns HTTP 400
`invalid-json-body` before a claim or provider interaction; the fixed message
names both accepted values. During the approved overlap below, an omitted field
selects only the frozen legacy protocol. After its scheduled removal, missing
intent also returns `invalid-json-body`; the explicit-intent contract has no
implicit default.

Reuse #3062's unchanged `owner_commands::parse` predicate: trim leading
whitespace, then inspect the first whitespace-delimited token for a leading
slash and no further slash. “Command-shaped” below means that existing
predicate, not a new command grammar or allowlist.

| Intent | Command-shaped | Admission and routing |
| --- | --- | --- |
| `text` | No | Exact ordinary text; retain caller idle policy and transport. |
| `text` | Yes | HTTP 409 `prompt-text-command-unsupported`; send nothing. |
| `command` | No | HTTP 400 `invalid-command-input`; send nothing. |
| `command` | Yes | Existing authenticated guarded owner command path. |

The new semantic errors have fixed actionable messages, respectively: “Put
ordinary text before the slash expression, or explicitly select command intent”
and “Command intent requires a leading single-component slash token; select
text intent for a path or ordinary prompt.” Never echo the prompt. For a valid
keyed request, claim the original payload before semantic admission and persist
a terminal `refused` receipt with that code. Lookup and identical POST recover
it without dispatch. Unkeyed requests use the same HTTP status/code without a
receipt. Existing authentication, identity, blank-input, size and unsafe-control
validation stays authoritative; new refusals never replace a retained outcome.

Examples: `/login why is authentication needed?` with text intent gets the new
text refusal; with command intent it enters existing login/command checks, never
a guessed prose branch. `Explain /login` and quoted `"/login"` are ordinary text.
`/uploads/example.txt caption` and `/ns/cmd` remain ordinary text under #3062;
command intent for either gets `invalid-command-input`. `/model
/uploads/example.txt` remains command-shaped. No filesystem lookup is needed.

## Existing policy boundaries

Text intent never forces idle or changes to command intent. Ordinary text with
false/omitted `require_idle` keeps existing busy-reply and queue behavior; true
keeps guarded delivery. Command intent always uses the live TUI with effective
`require_idle=true`, even when false/omitted was requested. Retain runtime,
provider-conversation and account fences, pending-ingress and empty-composer
checks, single-line/control restrictions, deadlines, confirmation and cleanup.
Commands never enter a model turn.

Keep the context denylist unchanged: Claude `/clear`, `/new`, `/reset`,
`/resume`, `/branch`, `/teleport`, `/tp`; Codex `/new`, `/clear`, `/resume`,
`/fork`. Arguments cannot bypass an exact denial; bare prefixes that can
complete to denied commands still get HTTP 409
`send-context-command-unsupported`. Keep the consumer terminal-guard mirror.
Text intent cannot bypass it because command-shaped text never reaches a
provider.

Only the existing sealed authenticated owner-command origin admits exact Claude
`/login` under authentication quarantine. Intent is not an origin credential.
Added prose/arguments are not exact recovery and stay refused in quarantine;
text intent never grants recovery. Quarantine remains until existing recovery
observes success. Codex `/login` outside quarantine retains
`codex-account-login-managed`; quarantined Codex input retains `auth-quarantined`.
Generic send, raw input, notifications, peer/model callers, account switching,
login/logout RPC refusals and authentication lifecycle remain unchanged.

## Receipts and bounded cross-host rollout

Fingerprint original exact text, explicit intent and requested idle flag before
command trimming or forcing idle. A changed intent gets HTTP 409
`request-id-conflict`, just as changed text/idle does. Never upgrade an attempt's
intent during retry. An intentional edit after definite refusal uses a new key;
unknown attempts keep their original key and reconcile. Keep receipt identity,
lookup URL, envelopes, outcomes, positive command-UI proof and at-most-once/no
blind replay rules. The public receipt needs no intent field: the original
request/outbox and fingerprint bind it.

Independently deployed edges, host daemons and lagging native clients cannot all
switch atomically. An approved repository newest-only policy exception for
cross-host online protocols permits exactly one adjacent-release overlap, at
most two named adjacent releases. It is limited to this online protocol and
does not authorize indefinite dual forms or unrelated compatibility paths.

Advertise `data.capabilities.owner_prompt_intent: "v2"` on the existing
`GET /healthz` response. The edge selects the explicit-intent form only for a host
advertising that exact value. Missing/unrecognized capability is not support;
old-host submissions retain the frozen pre-intent behavior and must not be
presented as intent-enforced. Do not retry a failed intent-bearing POST using an
intent-less body. An attempt already bound to intent must wait for a supporting
host rather than silently lose its intent or change its fingerprint.

During the overlap, the daemon accepts both the current intent-less
`/sessions/{id}/prompt/v2` body with frozen existing behavior and the intent-bearing
body with the new semantics. The older `/sessions/{id}/prompt` route also remains
during this bounded interval; do not remove it in the initial migration.
Legacy inference and fingerprint computation are confined to that temporary
protocol path. Explicit-intent attempts never enter it. Keep all existing
context/login/quarantine guards on the legacy path as well.

Before the implementation rollout can merge, the release owner must record the
exact adjacent release pair, cross-host justification, concrete first-disallowed
release and hard calendar expiry required by
[repository policy](../../../../DEVELOPMENT.md#current-contracts-in-this-repository).
This design does not invent release numbers or dates. No rolling expiry or
automatic extension is permitted. The release owner schedules a separate removal
step once every supported host reports v2, within that release/date fence; a
lagging host must be upgraded or withdrawn from prompt service before expiry,
not extend the window. Remove both old POST forms and their inference/fingerprint
code in that step. The explicit-intent body then becomes the sole form.

The implementation changes all supported caller sources together, while the
approved deployment interval accommodates installed clients:

1. Inventory both prompt POST routes, request constructors, tests, examples,
   proxies and persisted outboxes. The web console and Salon consumer owners
   agree on explicit command choice and typed error UI. Ordinary automated
   producers choose text explicitly, never infer commands from a slash.
2. Update every producer to select the intent-bearing body using the capability
   signal. Preserve only the two frozen old POST forms specified above during
   the approved interval; remove their handlers/body types and examples in the
   separately scheduled removal step. No new route version, alias or indefinite
   missing-field default.
3. Update request/proxy types, outbox persistence, retry identity and refusal
   rendering together. The edge assigns `intent: "text"` (the composer default)
   to legacy unsent outbox rows lacking intent. A command-shaped row receives
   `409 prompt-text-command-unsupported` and appears as a normal refused row
   with Discard/Retry actions. Never silently upgrade it to command; sending it
   as a command requires the explicit action and a new attempt after refusal.
4. Settle in-flight attempts at each caller's transition and retain uncertain
   IDs for lookup. Existing claims/receipts remain immutable evidence; never
   reinterpret an old payload/key as an intent-bearing attempt. Unknown stays
   unknown without sufficient proof. New-form proof/reconciliation uses the
   new tuple exclusively; legacy replay uses only its frozen tuple until removal.
   After removal, pre-transition attempts are lookup-only and never re-POSTed.
5. Track host capability adoption and native/edge consumer rollout. At removal,
   invalidate stale bundles and reject old forms before input. Keep receipt
   lookup available for retained evidence; do not fabricate or replay attempts.

Agreement is required from the owner prompt API maintainer, web console consumer
owner, and Salon composer/outbox and terminal-guard consumer owner. The steward
and lead route those decisions and designated review; record agreement and
review, plus owner sign-off on the explicit command action, before opening an
implementation issue. This design PR performs no migration.

## Implementation touchpoints

Paths below are relative to `crates/agent-session`, except consumer modules.

| Owner | Files/modules affected in implementation |
| --- | --- |
| HTTP admission | `src/serve/prompt_receipts.rs`: required intent, original-payload claim and routing; put new logic in a focused child module. |
| Command origin | `src/serve/prompt_receipts/owner_commands.rs`: reuse unchanged shape classifier; create sealed command origin only for command intent. |
| Request identity | `src/serve/prompt_receipts/store.rs`, `reconcile.rs`, `client_message.rs` and tests: update payload-fingerprint creation and confirmation dependencies together; retain key/receipt identity. |
| Capability and bounded protocol path | `src/serve.rs` health projection and routing plus focused child helpers: advertise capability, retain old forms only for the approved overlap, and remove them separately. Baselined files cannot grow; new logic belongs in new modules. Keep internal structured prompt/notification functions. |
| HTTP fixtures | New focused module under `src/serve/tests/`; migrate request bodies in `auth_quarantine.rs`, `owner_command_ui.rs`, `prompt_receipts.rs`, `native_busy_reply.rs`, `native_reconciliation.rs`, `codex_prompt_outcomes.rs`, Claude acknowledgement submodules and other prompt callers found by inventory. Baselined test files must not grow. |
| Canonical docs | `docs/specs/serve-prompt-v1.md`, `serve-api-v1.md`, `serve-observation-v1.md`, `serve-terminal-v1.md`, `native-busy-replies.md`, and `docs/runbooks/serve-daemon.md`: document capability, bounded overlap and later sole contract. |
| Consumer owners | Web console/Salon request type, HTTP client/proxy, composer action, outbox schema/migration/retry, refusal rendering and tests. Preserve the separate terminal context guard. Consumer owners confirm exact file inventory before implementation. |

## Synthetic HTTP test plan

Use the in-process HTTP router and existing synthetic provider/control-plane/TUI
fixtures. Assert HTTP status/code, receipt identity/outcome and recorded
paste/Enter/native-turn counts. Capture meaningful boundary regressions before
implementation; no running user session or shared-device probe is needed.

- Null/invalid intent and unknown fields: schema failure, no claim or provider
  input. Missing intent/old-route POST keep frozen behavior only during the
  approved interval, then fail schema/route validation without input. Exercise
  health capability advertisement and edge selection, keyed and unkeyed bodies.
- Both providers: known command plus prose as text gets the new terminal refusal
  with zero provider input; the same bytes as command enter current guards,
  without asserting a provider argument grammar or unsupported UI success.
- Ordinary text, quoted slash, synthetic upload path/caption and `/ns/cmd` keep
  ordinary routing. Busy Claude with idle false/omitted keeps queue admission;
  true keeps `busy`. Command intent for ordinary text/path gets the shape error.
- `/status`, `/model` and a synthetic command with a path argument force guarded
  TUI routing for all requested idle values. Retain busy/draft/identity fences,
  positive command-UI receipt and unknown-observation tests, with no model turn.
- All denied commands, arguments and bare prefixes keep the context refusal for
  command intent; text intent gets the text refusal with zero writes. No mirror
  denylist change is required.
- Synthetic quarantine: retain exact authenticated Claude login admission and
  recovery state, non-exact/text login refusal, unauthenticated/generic/peer
  refusal, managed Codex login refusal and quarantined Codex refusal.
- Identical tuples replay concurrently and after daemon restart with at most one
  dispatch and stable lookup. Changed intent or requested idle conflicts even
  when command idle would be forced. Legacy keys never become new-form attempts;
  after removal they stay lookup-only. Retain unknown reconciliation and
  changed-incarnation/account fences.
- Consumer tests prove explicit choice, persisted intent/retry, legacy unsent
  rows assigned text, intact forwarding and normal Discard/Retry refusal UI.
  A command-shaped migrated row never silently becomes a command.
- Rollout fixtures exercise old/new hosts and lagging clients within the exact
  approved release/date fence, no retry downgrade, and removal of both legacy
  POST forms once all hosts report v2. No live host/version probes are needed.

## Untouched

This PR changes documentation only. No provider inventory/version probes;
no change to #3062's first-token rule, denylist, login handling, quarantine
lifecycle, command-UI recognition or generic send/terminal policy. Initial
creation prompts and internal notifications retain separate contracts. The
follow-up changes owner intent admission, request identity and required callers;
ordinary provider transport, acknowledgement, busy replies, cleanup and account
controls retain current semantics. Any later native UI acceptance is separately
scheduled by its owner.
