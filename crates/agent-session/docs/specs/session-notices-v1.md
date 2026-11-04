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
producer-receipt-backed native notice gets the separate machine role, apart from the
display-only [mailbox read event](mailbox-read-events-v1.md) recorded by the daemon
and never delivered. Ordinary
human messages with a valid header retain their user role. Unmarked mailbox
reminders retain their existing filtering behavior.

The proposed [history row class contract](history-row-classes-v1.md) defines
the narrowly anchored, display-only recognition of stored unmarked text.
That design is not implemented by the codec described here and rewrites no
transcript. It explicitly supersedes reminder filtering only in its future
display projection; native receipt admission stays authoritative.

The language-neutral [fixture corpus](../../tests/fixtures/session-notices-v1.json)
contains exact JSON strings. Each case has `name`, `input`, and `notice`;
`notice` is null for unclassified text, otherwise it contains `kind`, `source`,
`id` (null when absent), and the exact `body`. Consumers can copy these public
fixtures and run them without Rust. Rust's `agent_session::notice` exports
`NoticeEnvelope`, `parse_notice`, `ParsedNotice` and `NoticeError`.

## CLI and delivery

The [summary writer design](#summary-writer-design) below extends this CLI in
a later implementation slice. The current command examples use today's body
input form.

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
probe: `2.1.296` through `2.1.318` inclusive. The launch strictly parses
`<major>.<minor>.<patch> (Claude Code)` from `--version` and compares
numerically. An unreadable or unparseable version is unsupported. The probed
version is recorded in the module bootstrap; the module and readiness compare
the provider's reported version to it, and a provider row must carry the bound
version.

The `2.1.318` upper bound passed the module contract checks; a native run on
that release is pending before merge.

An earlier bound, `2.1.316`, rests on the module contract checks plus a
native run on that release, with the same hooks, API calls and environment
access as on `2.1.314`. The run showed: a gate-enabled fresh launch mounted the
module, a notice after an ordinary turn was delivered as a plugin-attributed row
with zero Enter presses and a repeated key was refused, a restart mounted a new
incarnation and delivered a fresh-key notice, and a gate-off launch recorded
`writers-disabled`
([#3136](https://github.com/sympoies/nils-cli/issues/3136#issuecomment-6541042426)).
A native run on `2.1.314` showed the same
([#3136](https://github.com/sympoies/nils-cli/issues/3136#issuecomment-6538469717)).
Those runs did not exercise other releases inside the range, long or non-ASCII
bodies, busy-turn delivery or Console rendering.

The range never widens at runtime. The upper bound is raised only by a
reviewed change that cites the module call-shape scan, `claude plugin validate`
and a native notice run on that exact release; moving the lower bound would make
older private ledger records unreadable and needs its own review. Sessions
started before an upgrade of `agent-session` lack the bootstrap field and stay
unattached until restarted.

The default-branch-only [scheduled release check](../../../../.github/workflows/claude-release-check.yml)
reads the official npm `@anthropic-ai/claude-code` latest stable release and
verifies the wrapper and matching Linux executable package against registry
SHA-512 integrity before an isolated temporary install. It runs the existing
call-shape and `claude plugin validate` contract tests with private home/config,
XDG and temporary directories, no inherited credentials, settings, plugins or
runtime homes, and no updater. Candidate discovery, compilation, installation
and execution run in a read-only job. A separate fresh runner holds provider
write permissions and consumes bounded JSON evidence only; it rechecks source
and registry metadata and regenerates the allowed range-only patch from trusted
source before comparing the artifact. It never executes the candidate.

In-range releases create nothing. A newer failing
candidate upserts one version-keyed issue; a newer passing candidate prepares
one version-keyed draft PR changing only the maximum and directly necessary
expectations and prospective docs. Exact source, version, integrity, commands,
exits and run artifacts accompany the candidate. Existing human edits, review
activity, ready/closed decisions and occupied branches are preserved; candidate
creation failure is actionable in the retained report and failure issue. First
failure evidence and candidate branches stay immutable; later runs reuse the
version-keyed records and retain fresh evidence in their run artifacts.

**Contract success never claims native success.** Each future candidate starts
with **Native acceptance: PENDING**. Exact-version native notice acceptance and
reviewed merge gates remain required before extending the range. The check does
not merge, release, deploy or widen a running module's range. Its schedule
reduces detection delay but cannot guarantee beating every external auto-update;
it does not define host pinning or update policy.

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

Pre-claim `notice-native-unavailable` errors carry a stable `error.details.reason`
and include that reason in the text error. The reason contains only a fixed label:
no notice text, request values, nonce, provider binding or filesystem path.

| Reason | Refusal |
| --- | --- |
| `baseline-unprovable` | The provider transcript cannot be safely baselined. |
| `listener-open` | The native endpoint could not be opened or its offer published. |
| `authorization-timeout` | The authorization exchange exhausted the original send budget. |
| `authorization-connection` | The authorization connection failed or closed early. |
| `authorization-protocol` | The incoming HTTP/JSON exchange is invalid. |
| `authorization-payload` | The authorization body does not match its required shape. |
| `authorization-path`, `authorization-schema`, `authorization-operation` | The request path, schema or operation does not match. |
| `authorization-binding`, `authorization-nonce` | The request does not match the offered identity or nonce. |
| `authorization-surface`, `authorization-composer-not-empty`, `authorization-cursor` | The provider cannot attest an empty terminal composer at cursor zero. |
| `readiness-unavailable`, `readiness-changed` | Readiness is unavailable or differs when rechecked before claim. |
| `claim-unavailable` | The native ledger cannot admit the claim. |
| `claim-ledger-full` | The live claim cap remains full after eligible finished claims are archived. |

Runtime setup and private-file access use operation-specific labels under
`private-directory-*`, `private-file-*`, `socket-*`, plus `incarnation-invalid`,
`runtime-missing`, `module-parent-missing` and `*-serialize`. These diagnostics do
not relax file ownership/mode checks or authorize retry. After consumption, the
existing `notice-native-unconfirmed` result and retained claim remain authoritative.

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

### Native claim lifecycle

Private attestations live outside managed session directories so history survives
session deletion/archive. The live ledger permits 1,024 claims, reserving two
4-KiB record slots per claim in an 8-MiB data budget. Successor segments add one
bounded header slot. Unfinished and `unknown` claims remain live. At capacity,
a writer seals age-eligible submitted/dropped claims before admitting another
claim. If none can leave the live set, `claim-ledger-full` refuses before key
consumption; neither that reason nor storage failure permits plain input.

Finished claims become eligible after one hour. A valid terminal timestamp is
used when available. For an undated/invalid/future timestamp, a sweep persists
a conservative first-observed time; eligibility starts one hour after that
observation. Later sweeps retain that time. Archival runs only when admission
would exceed the live cap, under the original send deadline and a stable private
`store.lock`; readers take its shared lock. No background process is required.

The archive is an append-only set of sealed segments in `history/claude-notices`.
The genesis `ledger.jsonl` and each retired successor retain their exact bytes;
no record is truncated, deleted or rewritten. A successor copies the still-live
records exactly. Immutable manifests link predecessor manifests, sealed data and
compact key-index segments by SHA-256; the successor header binds the sealed
segment and its index. The directory stays owned `0700`; live data, index,
manifest, pointer and lock files stay owned regular `0600` files. Retired data,
including genesis, is sealed `0400`. All reads reject symlinks and wrong owners.

Each compact index permanently retains domain-separated hashes of the managed
session/caller-key pair and operation identity. Admission checks committed
indexes and live claims before consuming a key. Archival, session deletion or a
new runtime cannot make a retained key reusable. Attestation reads verify the
archive hashes, merge identical retained copies only, and keep the exact
claim/terminal/physical-provider-row checks. A conflicting or altered copy is
refused; this lifecycle adds no text-based notice classification.

Successor data, index and immutable manifest files are fsynced before an atomic
`current.json` publication. Unreferenced preparation files confer no claim or
key authority and remain untouched after interruption. Before publication, the
prior committed segment remains authoritative; afterward, all referenced
objects are durable. A durable `layout.json` marker precedes any unique record
in the successor, so missing committed metadata cannot select the genesis path.
A first publication interrupted before that marker contains copies only; the
next writer completes the marker before writing. Corrupt referenced state fails
closed, and an unfinished operation can finish against its unchanged claim hash.

After the pointer is durable, the writer seals the retired segment before any
unique successor write. If interrupted before sealing, the next writer verifies
the exact published tip and completes its seal; readers reject writable archives.
The `0400` guard rejects new older-writer opens, but cannot revoke open descriptors.

Every native notice writer sharing one state directory must upgrade together.
Stop every `serve` daemon and any `agent-session notice` process on the old binary
for that state directory before the first new-binary write, including queued
notice writers. Otherwise a retired segment can change and both native notices
and Claude History reads fail until repair. Independent state directories can
activate separately. Broker/proxy helpers and provider sessions do not write
this ledger. Use the reviewed version for archive-aware History reads as well.
The current implementation selects one live segment, with no dual writes or
older-path fallback after publication. No live-state migration or release is
part of fixture validation.

Initial prompts can explicitly use the same encoder:

```bash
agent-session start --agent claude --prompt-file prompt.txt \
  --notice-kind dispatch.bootstrap --notice-source scheduler --notice-id start-1
```

The same flags work with `--prompt` and `--prompt-stdin`, including
`--via-console`. Encoding occurs before launch; ordinary start prompts remain
unchanged. These flags classify only the supplied prompt, not native commands.
Keep `/compact` and other control actions in their existing command paths.

## Shared kind-to-label table

This table is the single design source for both summary rendering (#3228) and
anchored historical text recognition (#3226). The stored-title column records
text already present in transcripts; it is not an accepted writer alias.
A dash in the stored-title column means there is no additional title.

| Kind | Label | Stored title when different |
| --- | --- | --- |
| `mailbox` | Mailbox | — |
| `mailbox.wake` | Mailbox | Mailbox wake |
| `mailbox.read` | Mailbox read | — |
| `mailbox.alert` | Mailbox alert | — |
| `mailbox.bounce` | Mailbox bounce | — |
| `schedule.patrol` | Patrol | — |
| `schedule.pulse` | Pulse | — |
| `schedule.tick` | Schedule | — |
| `compact.reminder` | Compact reminder | — |
| `compact.resume` | Compact resume | — |
| `runtime.resume` | Resume | — |
| `context.continue` | Continue | — |
| `dispatch.bootstrap` | Dispatch start | — |
| `dispatch.role` | Role dispatch | — |
| `session.handover` | Handover | — |
| `monitoring.service` | Monitoring | — |
| `monitoring.mirror` | Monitoring mirror | — |
| `monitoring.delivery` | Monitoring delivery | — |
| `role.restart` | Role restart | — |
| `role.unavailable` | Role unavailable | — |

An unknown valid kind renders its exact kind token as the label. No namespace
prefix matching, title-casing, sender-selected label or consumer label registry
is added. Historical title matching yields only `row_class: "notice"`, not
invented kind/source metadata. In particular, the shared Mailbox label does not
require choosing between the `mailbox` and `mailbox.wake` kinds for raw text.

## Summary writer design

This section is a design for #3228, not a claim that `--summary` is released.
The later implementation adds `agent-session notice --summary <TEXT>`.
`TEXT` must contain 1..70 Unicode scalar values, with at least one non-whitespace
scalar. Reject CR, LF, U+0085, U+2028 and U+2029 so it occupies one logical line.
Reject other C0 controls and U+007F as well. A combining mark counts separately;
a non-BMP character counts once. The bound excludes the label and the separator
(a colon followed by one ASCII space).
Do not trim, normalize, split, truncate, strip a sender prefix or wrap a summary.

An invalid summary returns usage failure `notice-summary-invalid` (exit 2),
before body reading, target/session lookup, idempotency claiming or delivery.
Its bounded diagnostic describes the rule without echoing the supplied text.
Over 70 scalars is refused, never shortened. This does not change metadata
validation, the writer gate or transport-specific envelope size limits.

Render exactly `<Label>: <TEXT>` as the first body line, using the shared table.
One body source remains required. Given detail bytes `body`, the renderer emits
`<Label>: <TEXT>` + LF + `body`; the codec then emits the unchanged v1 header,
LF LF, and that rendered body. Empty detail bytes are permitted with a valid
summary and emit only `<Label>: <TEXT>` with no added LF. Otherwise the detail
bytes, including CRLF, leading spaces and final newlines, are untouched. Existing
parser double-envelope refusal remains; detail text is literal body content.

Summary formatting does not add wire fields or alter parser results for stored
bodies. No-summary calls do not invoke the renderer during migration.

The two external senders, a coordinator and a monitoring bridge, pass their own
unprefixed summary and unchanged detail body. Neither keeps its own first-line
renderer. Stored bodies are read as stored; readers do not require the new
first-line shape or relabel old bodies. Start-prompt flags are outside this
notice-command change.

### Mailbox byte invariance

The internal mailbox producer calls the same label/line renderer directly,
with kind `mailbox.wake`, source `agent-session` and no display id. It passes
`agent-session message inbox` for a fixed wake, or the existing spelled-out
count string plus `; agent-session message inbox` for a coalesced wake. It adds
no detail body or terminal LF. Header, separator and body must be byte-identical
to their current values. Preserve category order, punctuation and spacing.

Only this internal count producer is exempt from the external CLI's 70-scalar
summary-input bound: the existing quota-bounded count line can be longer.
Calling `notice --kind mailbox.wake --summary ...` does not gain that exemption.
It is not a new flag, delivery path or sender-controlled exception. The fixed
plain reminder and compact 51-column typed count reminder are untouched.
The [schema fixture](fixtures/history-row-classes-v1.json) includes exact fixed,
single-count and all-category wake envelopes, including a line over 70 scalars.

### Sender migration and retirement

The existing body-only writer is a bounded external-sender migration condition,
not a new writer mode. Until both external senders confirm migration, omitting
`--summary` leaves all output bytes unchanged. The removal slice makes summary
mandatory for every `notice` kind in the first release after both confirmations;
the coordinator assigns that release number. It deletes the no-summary branch
and its tests, migrates all supported callers, examples and completion assets,
and refuses omission as `notice-summary-required` (exit 2) before body reading
or delivery. No kind keeps an alternate writer form.

Owner: the release coordinator, with confirmations from both external sender
maintainers. Review date: **2026-11-30**. Hard expiry: **2026-12-31**. The
implementation slice must record the exact transitional occurrence as an
`external` retention row in `scripts/ci/own-compat-allowlist.toml`, linked to
[#3228](https://github.com/sympoies/nils-cli/issues/3228), naming the external
sender CLI integration versions and owner. That row
expires no later than the hard date and is deleted with the branch. The design
adds no audit exception before a runtime occurrence exists.

At review, record
both confirmations, the concrete first-disallowed release and its scheduled
removal; missing confirmation is escalated to the coordinator before expiry.
The 2026-11-09 consumer fixture target is independent of this sender review.

## Rollout and remaining work

All new writers require the explicit environment gate
`AGENT_SESSION_NOTICE_WRITERS=1` in the CLI caller environment. Unset or any
other value returns `notice-writers-disabled` before reading a body or starting
a runtime. The default remains off until compatible readers and downstream
mirrors are deployed. This does not enable any writer globally. The internal mailbox producer uses
this gate; auto-resume integration is separate.

Deploy the spec/codec first, then compatible readers and mirrors, then enable
writers only in the intended caller environment. Roll back by unsetting the
writer gate; retain readers for notices already stored in history. Internal mailbox reminders call the same encoder directly with kind
`mailbox.wake`, source `agent-session` and no display id. The body is the
coalesced live unread-count reminder, for example `Mailbox: 1 handoff, 2 report;
agent-session message inbox`; it never
contains message bodies or recipient identity. Idle and working Codex retain
their existing start/steer paths; authenticated hook claims return the same
notice text. Supported terminal recipients use guarded notice delivery,
including the existing native Claude module. This preserves the mailbox's
single generation/incarnation claim across controller and hook paths.

The mailbox keeps its existing plain reminder only when the writer gate is
off or the receiver is known to lack notice support. Missing readiness on an
installed module is uncertain and defers; a failed or uncertain notice attempt
never authorizes a plain fallback or a second attempt. Attempt receipts retain
the exact envelope and `mailbox-notice.v1` format; positive observation can
resolve an uncertain notice, while absence keeps it parked. Historical prompt
attempts retain their original interpretation. No mailbox producer shells out
to `notice`. Auto-resume integration remains separate.

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
