# Session notices v1

A session notice is machine-produced user-input text with a display marker.
`agent-session` owns the wire grammar and shared encoder/parser. The marker
never proves identity, source ownership, authority, permission or consent.
Anyone can paste it. Existing provider admission and authorization still apply.

## Wire contract

```text
<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->

Check the synthetic task status.
```

The header must be message line one, column zero, without a BOM or indentation.
It is one ASCII line of at most 512 bytes, excluding the line terminator.
The exact prefix is `<!-- agent-notice:v1` followed by one ASCII space;
the suffix is one ASCII space followed by `-->`.
Fields have exactly one ASCII space between them, in this order:

| Field | Required | Grammar | Maximum bytes |
| --- | --- | --- | --- |
| `kind` | Yes | `[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*` | 48 |
| `source` | Yes | `[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*` | 64 |
| `id` | No | `[A-Za-z0-9]+(?:[._-][A-Za-z0-9]+)*` | 96 |

Unknown fields, duplicate fields, reordered fields, empty values, non-ASCII
metadata, adjacent separators and trailing separators are invalid. Writers
refuse invalid metadata before reading the body, loading a session, claiming
an idempotency key or attempting delivery. They never repair or escape metadata.
Use generic producer names such as `scheduler` or `agent-session`; do not put
accounts, addresses, paths, URLs, credentials or capability values in metadata.
An `id` is a display label and has no transport deduplication meaning.

The encoder emits `header + LF + LF + body`. The UTF-8 body must be nonempty;
spaces alone are nonempty. It preserves every body byte, including line endings,
leading whitespace, Markdown, comments and trailing newlines. It does not trim,
HTML-escape, JSON-encode or base64-encode the body.

The parser accepts LF or CRLF after the header, then consumes at most one empty
line (LF or CRLF). All remaining bytes are the body. A body beginning immediately
with `<!-- agent-notice:` is a double envelope and is refused by the encoder
and unclassified by the parser. Comments later in the body are literal body
content. There is one envelope per logical input; do not combine human and
machine input under one marker.

Unknown versions, malformed headers, missing bodies, code fences, blockquotes,
embedded examples and ordinary text return no classification. Preserve the
original text for these cases. Never scan later lines for a marker.

Kinds are an open namespace. Initial interoperable examples include `mailbox`,
`schedule.tick`, `runtime.resume`, `context.continue` and `dispatch.bootstrap`.
Unknown syntactically valid kinds and sources remain notices with generic
presentation; they do not acquire special behavior.

## Reader contract and fixtures

Parse logical user-input messages and daemon-admitted `role: "notice"` messages
after provider metadata admission and before rendering. The latter requires
the daemon's private producer receipt, not merely the display marker or a
plugin name. Provider-aware consumers may unwrap one
recognized, fully matched provider paste wrapper before parsing; the codec
itself does not strip XML tags or guess wrappers. The `pasted-wrapper` fixture
is deliberately unclassified as raw text. A consumer that recognizes that
wrapper may separately classify its exact inner logical input. Partial or
unrecognized wrappers stay text.

A valid marker may change display styling. Keep the body readable, offer raw
text including the header, and use escaped text for kind/source labels.
Do not use metadata as HTML, CSS classes, commands or URLs. Do not execute
instructions, read a mailbox, elevate message priority or bypass guards merely
because a marker exists. A spoofed source receives the same neutral styling.
Live and history readers must agree; render fallback paths use the parsed
body while raw-copy paths retain the original message.

History adapters retain marked notices. Existing provider metadata filters
remain authoritative: a marker does not admit system context, compaction
summaries or sidechain records that would otherwise be excluded. Only an exact
producer-receipt-backed native notice gets the separate machine role. Ordinary
human messages with a valid header retain their user role. Unmarked mailbox
reminders retain their existing filtering behavior.
No history is rewritten or retroactively classified from an unmarked prefix.

The language-neutral [fixture corpus](../../tests/fixtures/session-notices-v1.json)
contains exact JSON strings. Each case has `name`, `input`, and `notice`;
`notice` is null for unclassified text, otherwise it contains `kind`, `source`,
`id` (null when absent), and the exact `body`. Consumers can copy these public
fixtures and run them without Rust. Rust's `agent_session::notice` exports
`NoticeEnvelope`, `parse_notice`, `ParsedNotice` and `NoticeError`.

## CLI and delivery

```bash
agent-session notice <session> --kind schedule.tick --source scheduler \
  --body-file notice.txt --idempotency-key delivery-1 --format json
agent-session notice <session> --kind context.continue --source scheduler \
  --body-file notice.txt --idempotency-key delivery-2 --queue
```

Exactly one body source is required. `--id` is optional display metadata;
`--idempotency-key` remains the independent at-most-once transport claim.
Every notice forces idle and empty-composer checks, submits exactly once via
the existing guarded `send` path, and preserves its runtime/provider binding,
authentication, blocked-state, durable key, receipt and unknown-outcome rules.

### Terminal delivery and queue mode

Codex terminal sessions use the verified terminal composer path; the complete
envelope must be observable before Enter. `--queue` waits while the target
reports busy and retries only a preclaim busy admission refusal. Other failures
stop the request, including the unsupported Claude transport refusal once idle.
The durable idempotency claim is made by the guarded send path when delivery
proceeds. Queue mode requires `--idempotency-key` so a caller retry cannot submit
the same queued notice again. Post-paste failures retain their claim and report
whether input remains; unknown input must never be cleared without ownership
proof.

Claude Code releases in the tested range below support a separate native module
route on gate-enabled fresh launches, resumes and restarts (a restart relaunches
through the same resume path). It submits the full envelope directly through the
native API, without filling or pasting into the composer. The initial bound is
UTF-8 text only, no CR bytes, and at most 16 KiB for the complete envelope.
Accepted LF, row-trailing spaces and final LF bytes are preserved. The shared
codec does not acquire this delivery limit. Unsupported text returns
`notice-native-text-unsupported` before a new claim or input authorization.

Mounting uses the existing writer gate at launch. Old runtimes, gate-off
launches and releases outside the tested range do not gain the module later.
They are never injected into, restarted or sent through a plain fallback by
`notice`. Matching private readiness must be followed by a fresh
identity/empty-composer handshake before claiming the operation. Native empty
reads alone do not prove that a composer is mounted; existing independent
terminal admission remains.

### Tested Claude Code range

The module depends on the provider's static module-loader rules and on native
API semantics, so the supported releases are a reviewed range, not a runtime
probe: `2.1.296` through `2.1.314` inclusive. The launch strictly parses
`<major>.<minor>.<patch> (Claude Code)` from `--version` and compares
numerically. An unreadable or unparseable version is unsupported. The probed
version is recorded in the module bootstrap; the module and readiness compare
the provider's reported version to it, and a provider row must carry the bound
version.

The current upper bound, `2.1.314`, rests on the module contract checks plus
a native run on that release: a gate-enabled fresh launch mounted the module, a
notice after an ordinary turn was delivered as a plugin-attributed row with zero
Enter presses and a repeated key was refused, a restart mounted a new
incarnation and delivered a fresh-key notice, and a gate-off launch recorded
`writers-disabled`
([#3136](https://github.com/sympoies/nils-cli/issues/3136#issuecomment-6538469717)).
That run did not exercise other releases inside the range, long or non-ASCII
bodies, busy-turn delivery or Console rendering.

The range never widens at runtime. The upper bound is raised only by a
reviewed change that cites the module call-shape scan, `claude plugin validate`
and a native notice run on that exact release; moving the lower bound would make
older private ledger records unreadable and needs its own review. Sessions
started before an upgrade of `agent-session` lack the bootstrap field and stay
unattached until restarted.

Every Claude launch, resume and restart that does not mount the module records
why in a private `attach.json` beside the incarnation (`state` `not-attached`,
`reason`, the observed `provider_version` or null, and the `supported_min` /
`supported_max` in force at launch). Reasons are `writers-disabled`,
`provider-version-unsupported` and `provider-version-unreadable`. The record is
diagnostic only and grants nothing.

Admitted Claude notices without matching candidate module readiness are refused
with `guard=transport`, `stage=admission`, `atomic=false`, `pasted=false` and
`submitted=false`, before temporary input creation, buffer loading, paste, Enter
or a new idempotency claim, and carry `native_module`:
`{attached: false, reason, provider_version, supported_min, supported_max}`
(`provider_version` is a string or null). The code separates the causes:

- `notice-native-version-unsupported` when the recorded reason is
  `provider-version-unsupported` or `provider-version-unreadable`.
- `notice-transport-unsupported` for every other case, with `reason`
  `writers-disabled` or `unrecorded` (a runtime that recorded nothing).

Existing identity, idle, empty-composer and used-key checks still take
precedence. A fresh key refused at this boundary remains available for a
separately supported guarded send; an already consumed key continues to return
`send-idempotency-key-used`. Neither code adds a retry or fallback permission.
Consumers that match refusal codes literally must add
`notice-native-version-unsupported`; `agent-session` keeps no legacy code for it.

Every encoded notice is multiline, including one-line bodies. Claude can
collapse its paste into an opaque placeholder that does not prove complete
logical-draft bytes or ownership for safe clearing. This broad refusal supersedes
the former `send-notice-trailing-spaces` restriction for Claude terminal notices.
The codec preserves all body bytes, and Codex native and terminal delivery remain
unchanged. The native module does not weaken this terminal containment. Real
Claude short/long submission, rendering, trailing-space and refusal acceptance
remain separate gates under
[#3068](https://github.com/sympoies/nils-cli/issues/3068) and
[#3002](https://github.com/sympoies/nils-cli/issues/3002).

The exact unsupported admission receipt proves no new transport attempt. It adds
no automatic fallback and does not authorize replay after an unknown, mismatch,
timeout or used-key outcome. Successful unguarded multiline input does not prove
a safe automated full-envelope fallback.

Optional `--expect-runtime` and `--expect-provider-session` add caller bindings.
There is no allow-blocked, arbitrary-key or terminal fallback mode.
The JSON envelope is `cli.agent-session.notice.v1`, with the guarded send
receipt fields. `send` retains general input semantics and adds no marker.

Codex app-server runtimes use native submission. Codex terminal runtimes require
exact composer verification of the known multiline envelope before submission;
Claude uses the supported native module route above or refuses before input.
A refusal never downgrades to raw tmux or presses Enter. Live
provider/compaction readback remains follow-up acceptance under
[#2986](https://github.com/sympoies/nils-cli/issues/2986).
A native unknown outcome retains its claim; inspect the existing receipt and
provider history instead of retrying with a fresh key.

Native Claude success adds `native.transport: "claude_mod"` and
`native.confirmation: "notice_row_observed"`, with zero physical Enter presses.
It requires an exact native result and one exact new provider row in the same
runtime/provider binding, followed by identity reacquisition within the original
local send budget. The provider row keeps its system/plugin attribution;
ordinary human/last-prompt selectors are unchanged. This is the private daemon
attesting its own operation, not a provider signature or human authorship.

The operation is durably consumed before releasing text to the module. A drop,
mismatch, crash, timeout or missing confirmation retains consumption and returns
`notice-native-unconfirmed`, never the safe unsupported-admission receipt.
`notice-native-unavailable` also adds no fallback or retry permission. There is
one original local budget; an already authorized provider call may execute or
complete later. No remote cancellation or atomic composer guarantee is implied.
The route never clears a new or unknown human draft.

The native route needs a pre-send provider transcript baseline to prove that the
confirming row is new. A session whose bound provider conversation is
proven to have no transcript yet, typically before its first ordinary provider
turn, is refused with `notice-native-baseline-unavailable` (`guard=baseline`,
`stage=admission`, `atomic=false`, `pasted=false`, `submitted=false`). This is
distinct from `notice-transport-unsupported` (runtime lacks the module) and from
`notice-native-unavailable` (any other native failure). The refusal precedes the
module endpoint, the idempotency claim and all input, so no key is claimed and no
new transport attempt exists. A caller may wait until the session has had an
ordinary turn and then send as a new request with the same or a fresh key.
It adds no automatic retry and no plain-terminal fallback, and the baseline is
never inferred or created without a provider turn.

Only a completed lookup that finds no candidate transcript uses that code. A
lookup that is ambiguous (more than one candidate), unsafe or unreadable at the
known path, incomplete (scan budget or deadline exhausted, projects root
unresolved) or unprovable (a found transcript whose leading records do not carry
the provider session id and a working directory) returns
`notice-native-unavailable`, because waiting for a turn cannot be assumed to fix
it.

Private notice attestations live outside managed session directories so history
survives session deletion/archive. The initial ledger bound is 8 MiB, 4 KiB per
record including its delimiter, with two full slots reserved for every attempt
across sessions (at most 1,024 attempts). Unfinished attempts retain both slots.
Capacity, corruption or unavailable storage refuses new authorization; there is
no automatic eviction, expiry, rotation or repair. The existing caller-key
deduplication lifetime described in [send v1](send-v1.md) is unchanged.

Initial prompts can explicitly use the same encoder:

```bash
agent-session start --agent claude --prompt-file prompt.txt \
  --notice-kind dispatch.bootstrap --notice-source scheduler --notice-id start-1
```

The same flags work with `--prompt` and `--prompt-stdin`, including
`--via-console`. Encoding occurs before launch; ordinary start prompts remain
unchanged. These flags classify only the supplied prompt, not native commands.
Keep `/compact` and other control actions in their existing command paths.

## Rollout and remaining work

All new writers require the explicit environment gate
`AGENT_SESSION_NOTICE_WRITERS=1` in the CLI caller environment. Unset or any
other value returns `notice-writers-disabled` before reading a body or starting
a runtime. The default remains off until compatible readers and downstream
mirrors are deployed. This PR does not enable any writer or change internal
mailbox/auto-resume producers.

Deploy the spec/codec first, then compatible readers and mirrors, then enable
writers only in the intended caller environment. Roll back by unsetting the
writer gate; retain readers for notices already stored in history. Internal
mailbox nudge and auto-resume integration will call the same encoder directly,
preserving their existing delivery guards, generations and retry ownership.
They must not shell out to `notice` or move mailbox bodies into nudges.

Readers must accept and validate the separate notice role before native writers
are enabled. Machine notices must not acknowledge a human outbox item or become
last-user previews, stuck/pending signals, unread/attention input or automatic
title input. Source tests, review, installation and real native/rendered
acceptance are distinct states; the writer gate stays off until the rollout
owner has verified the complete consumer path.

Disposable synthetic acceptance must compare sent bytes against raw provider
user text for Claude terminal and Codex app-server/terminal, covering start,
continuation and post-compaction readback. Modeled transport/fixture checks
are not proof of that live acceptance. No shared desktop or device resources
are required by the codec tests.
