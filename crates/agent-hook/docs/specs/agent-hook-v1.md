# agent-hook v1 contract

Status: frozen for `sympoies/agent-runtime-kit#686` Lane A and extended with
the native WorkspaceLease v1 boundary for `sympoies/dsh-runtime-kit#56`.

Ownership: crate-local canonical specification for `nils-agent-hook` and the
`agent-hook` binary.

## Contract map

| Need | Canonical owner |
| --- | --- |
| Shared schemas, provider ingress, policy registry and writer liveness | This specification |
| DSH workspace binding, target-scoped authority and recovery | [Workspace leases](dsh-workspace-lease-v1.md) |
| DSH finish-line request and response operations | [Finish-line operations](dsh-finish-line-v1.md) |
| Rule precedence, interaction lanes and release skew | [Evaluation and degradation](evaluation-and-degradation-v1.md) |
| Activity helper selection, setup rollback and governed recovery | [Provider setup and recovery](provider-setup-recovery-v1.md) |
| Acceptance RPC ordering and provenance | [Finish-line acceptance](finish-line-acceptance-v1.md) |
| Data policy service | [Data policy](data-policy-v1.md) |

## Purpose and boundary

`agent-hook` is the sole owner of nils-cli-managed Codex and Claude hook
registrations and the policy engine behind the external DSH runtime bundle.
Provider-native hooks remain lifecycle ingress, but their owned commands
contain only `agent-hook dispatch --product <provider>`; policy rules never
live in provider configuration. DSH is registered by `dsh-runtime-kit`, not by
`agent-hook setup`. The CLI does not manage unrelated hooks, execute
config-defined programs, weaken lower-level transaction/privacy rules, or claim
native setup ownership for providers without a compatible runner.

## Files and limits

The CLI resolves only absolute XDG roots and rejects relative roots.

| Role | Default | Limit / mode |
| --- | --- | --- |
| user config | `${XDG_CONFIG_HOME:-$HOME/.config}/agent-hook/config.toml` | 64 KiB, regular file, no symlink |
| policy bundle | config-selected absolute path, normally below `${XDG_DATA_HOME:-$HOME/.local/share}/agent-hook/policies/` | 1 MiB, regular file, no symlink |
| runtime state | `${XDG_STATE_HOME:-$HOME/.local/state}/agent-hook/` | directories `0700`, secret files `0600` |
| provider payload | standard input | 1 MiB |
| workspace-lease request | standard input | 256 KiB, strict duplicate-free JSON |
| workspace-lease state | state-owned per-worktree JSON | 512 KiB, `0600`, keyed private identity |
| workspace-recovery request | standard input | 64 KiB, strict duplicate-free JSON |
| workspace-recovery result | standard output | 192 KiB result data, 512 inspected worktrees, 2,048 inspected dirty paths, 16 KiB per path; arrays carry omitted counts when projected |
| trace | state-owned JSONL | 256 entries, 256 KiB, redacted metadata only |
| rule inventory | one policy bundle | 512 rules, unique IDs |
| reason metadata | one aggregate decision | 64 reasons, 256 bytes per code/message |

Unknown fields are rejected in every persisted/input schema. Service response
envelopes are the compatibility exception: consumers accept additive envelope,
error, and command-result metadata within the same schema version while still
requiring the exact schema version, success state, and mandatory result fields.
Identifiers are ASCII kebab-case, at most 128 bytes. Policy/config digests are
lowercase `sha256:<64 hex>` values over canonical serialized bytes.

## Versioned schemas

Every persisted/input schema has a literal `schema_version` and uses strict
unknown-field handling. Service responses use the additive compatibility rule
above.

- `agent-hook.config.v1`: selected policy path and digest, per-provider mode,
  and rule overrides. It cannot contain commands, bearer material, environment
  assignments, executable paths, or recovery state.
- `agent-hook.policy.v1`: bundle ID/version and ordered typed rules. A rule has
  stable ID, products, events, optional matcher, priority, mode, failure
  posture, override class, and one built-in capability binding. The concrete
  TOML serialization is frozen below.
- `agent-hook.normalized-request.v1`: request ID, product, canonical event,
  optional matcher, bounded target/command/snapshot digests, and public boolean
  facts. Raw prompts, tool input, command strings, paths, session IDs, mailbox
  bodies, environment values, and provider payload fragments are never stored.
  The provider's own Stop re-entry marker (`stop_hook_active`) is projected into
  one public boolean fact. It is trusted in exactly one direction: `true` may end
  a turn that is already looping, and it can never grant authority, release a
  claim, or downgrade a proven owner.
- `agent-hook.dsh-ingress.v1`: strict DSH-to-policy transport for one native
  extension event. Version 1 carries exactly `event`, bounded `call_id`, an
  absolute `cwd`, and a `tool` object with bounded `name` and object-valued
  `arguments`. Unknown fields are rejected at the root and nested tool object,
  so v1 explicitly forbids `subject`.
- `agent-hook.dsh-ingress.v2`: the current DSH transport. It retains the v1
  fields and adds one strict `subject` containing bounded `session_id`, positive
  `turn` and `step`, an absolute `agent_docs_state_home`, and an optional
  absolute `agent_docs_home`; v2 requires this complete subject. V1 remains
  accepted for compatibility, but typed
  identity-dependent DSH policy groups fail closed without the v2 subject.
- `agent-hook.dsh-ingress.v3`: the DSH lifecycle transport. It accepts exactly
  `agent/pre-step` with a required UTF-8 prompt of at most 64 KiB, a positive
  step, and an optional closed-enum session-start source. The accepted values
  are `startup`, `resume`, `clear`, `compact`, and `observed`; the first four
  are rc.7 lifecycle sources and `observed` is the adapter-derived value for a
  late or hot-reloaded attachment. It also accepts
  `agent/turn-stopping` with neither prompt nor step/source. Both shapes retain
  the v2 subject identity and absolute agent-docs roots. Unknown or cross-event
  fields are rejected. The runtime bundle continues to use v2 for tool ingress.
- `agent-hook.dsh-ingress.v4`: the DSH post-tool transport. It carries the same
  strict call, subject, cwd, and object-valued tool identity as v2 plus exactly
  `result.is_error: boolean`. `false` normalizes to `PostToolUse`; `true`
  normalizes to `PostToolUseFailure`. Candidate value, content, error objects,
  stdout, and stderr are neither accepted nor persisted.
- `agent-hook.dsh-ingress.v5`: the prerequisite-bound DSH pre-tool transport.
  It retains the strict v2 call, cwd, session, turn/step, tool name, and
  arguments, and additionally requires bounded `subject.agent_id`,
  `subject.workspace_generation`, `tool.definition_id`, and
  `tool.prerequisite_receipt`. The receipt is re-resolved against the current
  agent-docs catalog and exact execution binding; verification creates no
  activation. V5 fields are rejected on every older ingress version.
- `agent-hook.finish-line.{open,begin,run,register,admit,observe,verdict,stop,status}.v1`: strict DSH lifecycle
  requests for the native execution-owned finish line. Their
  command result schemas use the matching
  `agent-hook.finish-line.<command>-result.v1` name and the normal
  `cli.agent-hook.finish-line-<command>.v1` service envelope.
- `agent-hook.finish-line.{quiesce,release}.v1`: strict internal DSH cleanup and
  session-retirement requests. Their matching result and service schemas follow
  the same naming rule, but both commands are intentionally absent from public
  help and completion.
- `agent-hook.workspace-lease.{bind,begin,complete,renew,release}.v1`: strict
  native WorkspaceLease provider requests. Matching results use
  `agent-hook.workspace-lease.<command>-result.v1` inside the normal
  `cli.agent-hook.workspace-lease-<command>.v1` service envelope.
- `agent-hook.workspace-lease.{resolve,bind,begin,complete,renew,release}.v2`:
  strict target-scoped WorkspaceLease provider requests. Matching results use
  `agent-hook.workspace-lease.<command>-result.v2` inside the same service
  envelope. The protocol generation is selected by the request schema; a
  declared `version` that disagrees with that schema is rejected as an
  unsupported mixed-version request.
- `agent-hook.workspace-recovery.inspect.v1` and
  `agent-hook.workspace-recovery.verify-handoff.v1`: strict read-only dirty
  checkout inspection and exact clean managed-worktree handoff verification.
  Both return `agent-hook.workspace-recovery.result.v1` inside the matching
  `cli.agent-hook.workspace-recovery-<command>.v1` service envelope.
- `agent-hook.normalized-decision.v1`: aggregate action, ordered reason codes,
  optional bounded context or replacement, shadow observations, config /
  policy digests, and two optional audit fields: `enforcement` (`block`,
  `advise`, or `context`; absent for an allow, warn, or transform aggregate)
  and `downgraded_by` (the
  `<config path> [overrides.<rule id>]` of the `advise` override that
  projected a block; absent when nothing was downgraded).
- `agent-hook.trace.v1`: timing, rule IDs, disposition classes, and digests;
  never raw payload, paths, identities, message content, or capabilities.
- `agent-hook.setup-plan.v2`: product, install/remove operation, owned
  event/matcher groups, role-tagged before/after digests or file absence for
  every provider source, exact plan digest, drift state, and whether apply is
  permitted. Provider hook argv content is omitted.
- `agent-hook.doctor.v1`: product status (missing, compatibility-only, `dual`,
  `drifted`, `converged`, `unsupported`, or `unrelated`), owned counts,
  compatibility residue count, digests, policy availability, recovery health,
  registration owner, and whether direct dispatch is supported.
- `agent-hook.inventory.v1`: ordered public rule metadata and effective mode;
  capability parameters and private paths are omitted.
- `agent-hook.recovery-challenge.v1`: random challenge ID, exact product/event,
  target/command/snapshot digests, requested rule IDs, scope, issue/expiry time,
  state revision, and a digest-bound `agent-hook.recovery-manifest.v1` projection
  of every enforceable rule for that product/event.
- `agent-hook.recovery-capability.v1`: capability ID, challenge digest, exact
  binding, scope, expiry, nonce, state revision, and authorization proof digest.
  The capability file is the bearer and is never printed.
- `agent-hook.owner-liveness.v1`: only classification (`active`, `stale`,
  `orphaned`, `unknown`, or `unclaimed`), semantic conflict class, and
  content-free reason codes.
- `agent-session.observation.v1`: the centralized control-plane event plane.
  Dispatch records exactly one terminal event per invocation — including
  outcomes that fail before normalization or policy load — plus non-terminal
  events for a degraded lane, a terminal Stop exit, a crossed broker release
  boundary, and a discarded helper override. Recording is unconditional rather
  than gated behind `--trace`, is independent of any daemon, broker, or
  capability, and is best-effort so it can never block a session. Schema,
  privacy budget, spool bounds, and the diagnostic bundle are normative in
  `agent-session/docs/specs/control-plane-observation-v1.md`.

Service JSON uses the workspace envelope: `schema_version`, `ok`, then `data`;
failures contain `error.code`, `error.message`, and
optional redacted `error.details`. Text is the human default except `dispatch`,
whose default is provider output, and `finish-line` plus `workspace-lease`,
whose default is service JSON. `--format json` always selects the service
envelope.

## Provider normalization and rendering

Products are `codex`, `claude`, `dsh`, and `hermes`. Codex, Claude, and DSH are
enforceable. DSH setup reports `unsupported` because the out-of-tree runtime
bundle owns registration. Hermes validates and evaluates shared policy but
reports `unsupported` for native setup until a compatible runner exists.

Supported canonical events are:

- Codex: `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `PreToolUse`,
  `PostToolUse`, `PostToolUseFailure`, `PreCompact`, `PostCompact`,
  `SubagentStart`, `SubagentStop`, `Stop`, and `Interrupt`.
- Claude: `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `PreToolUse`,
  `PostToolUse`, `PostToolUseFailure`, `PreCompact`, `PermissionDenied`, `Stop`, `StopFailure`,
  `Notification`, `SubagentStart`, `SubagentStop`, `Elicitation`, and
  `ElicitationResult`.
- DSH: `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `UserPromptSubmit`,
  and `Stop`, normalized respectively from native `tools/pre-execute`,
  `tools/post-execute`, `agent/pre-step`, and `agent/turn-stopping` ingress.

Codex `Interrupt` follows the upstream `interrupt.command.input` schema
(openai/codex `rust-v0.162.1`; identical at `rust-v0.150.0`, which introduced
it) and has no matcher input. Its output admits only `systemMessage`, so
policy may register it for `decision.allow.v1`, `agent-session.activity.v1`,
and runtime-kit handlers only; block, context, and transform capabilities are
rejected as `policy-capability-event-unsupported`. Activity normalization
records an observed `stop_observed` turn event, the same observation as Codex
`Stop`, and requires both `session_id` and `turn_id`; a missing or invalid
identifier fails closed with `provider-hook-correlation-missing` or
`provider-hook-identifier-invalid`, and a closed block renders as
`systemMessage`.

The Codex/Claude/Hermes adapter accepts the event from `--event` or the
provider's documented `hook_event_name`/`event` field and rejects a mismatch.
The DSH adapter accepts strict `agent-hook.dsh-ingress.v1`, v2, v3, v4, or v5;
the runtime bundle uses v2 before ordinary tools, v5 when a pending prerequisite
must cross pre-tool policy, v4 after tools, and v3 for agent lifecycle events.
An optional
`--event` must equal its native event string before that string is mapped to
the canonical policy event. Matcher values are normalized only from documented
fields:

| Provider events | Matcher input field |
| --- | --- |
| Codex/Claude `SessionStart` | `source` |
| Codex/Claude `PermissionRequest`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure` | `tool_name` |
| DSH `PreToolUse`, `PostToolUse`, `PostToolUseFailure` | `tool.name` |
| Codex/Claude `PreCompact`; Codex `PostCompact` | `trigger` |
| Codex/Claude `SubagentStart`, `SubagentStop` | `agent_type` |
| Claude `Notification` | `notification_type` |
| Claude `Elicitation`, `ElicitationResult` | `mcp_server_name` |
| Claude `StopFailure` | `error` |

Correlation identities are read from the provider's own fields and projected
into opaque local ids; raw provider values never leave the adapter. The session
identity comes from `session_id`, or `session_key` when that is the provider's
name for it. The turn identity comes from `turn_id`, falling back to `prompt_id`
for Claude, which is the field Claude actually emits and which stays stable
across the turn's own `Stop` and idle `Notification` events.

Path-bearing mutation inputs are normalized before rule evaluation. `Write`,
`Edit`, and `MultiEdit` require exactly one non-empty `path` or `file_path`;
`NotebookEdit` requires `notebook_path`; and Codex `apply_patch` requires its
native `tool_input.command` string to contain a bounded, structurally complete
patch whose `Add File`, `Update File`, `Delete File`, and `Move to` directives
are all mapped. The undocumented `tool_input.patch` alias is not accepted.
Relative targets require an absolute provider execution directory. Missing,
ambiguous, malformed, oversized, or only partially mapped targets fail closed
as `provider-target-untrusted` rather than falling back to the execution
checkout.

Existing targets and the nearest existing ancestor of a new target
are resolved through one fail-closed binding resolver before ownership and
recovery evaluation. This includes final and intermediate symlinks: a path in
one checkout that resolves into another checkout is classified and bound to
the effective checkout, while dangling, cyclic, or otherwise ambiguous links
are rejected. Multi-target requests retain every distinct effective target for
owner-liveness evaluation and bind recovery to a deterministic target-set
digest. The v2 binding domain and ordinary canonical non-symlink single-target
digests remain unchanged. Raw target paths are never serialized in the
normalized request, decision, trace, or recovery artifact.

Owner-liveness evaluates every distinct target checkout plus the execution
checkout and returns the strongest result; an active foreign owner therefore
cannot be masked by a self-owned or unclaimed target.

DSH tool ingress preserves the complete object-valued tool arguments for policy
normalization. Native `write` and `edit` map `file_path`, and mutating
`str_replace_editor` commands map `path`, to an exact checkout-bound target.
Opaque Bash mutation uses the canonical repository target and the
`agent-session` shell-coverage contract; an exact file claim remains narrow.
Policy validation supports native allow/block admission and bounded model
context. DSH does not support policy argument transforms or retired file
handlers. Identity-dependent `dsh.policy.v1` groups require the complete
v2/v3/v4 subject.

Task 3.4 implements `agent-activity` and `operation-lifecycle`. Activity is
derived only from the normalized DSH subject and canonical event: it sends
provider/session/turn metadata, event kind, confidence, and digests to
`agent-session activity event`; prompt and tool arguments are never parsed for
that event. A managed activity identity requires both session and runtime ID.
When managed DSH policy binds the ingress subject to its authenticated
coordination owner, the runtime transports the original DSH provider session
only through the private `DSH_RUNTIME_KIT_PROVIDER_SESSION_ID` subprocess
environment. Activity normalization uses that value for provider correlation
while every other policy group continues to use the owner subject. The public
ingress schema is unchanged, and `agent-session` still rejects a value that
does not match the provider session already bound to the active runtime.

Operation lifecycle is one locked after-policy capability. An unmanaged process
does not run it. A managed process requires session ID, an absolute capability
file, and an absolute state root; the presence of any forwarded managed-session
selector makes an incomplete identity fail closed. Before a covered mutation, it obtains the
active claim, atomically writes owner-private targets, execution token, hashed
provider correlation, and deterministic idempotency keys, then invokes the
same-release sibling `agent-session work-context admit`. The post-tool v4 fact
drives the matching `complete --outcome pass|fail`. A repeated pre-tool identity
never authorizes a second execution; terminal post retries reauthenticate the
same idempotent completion and retain the original admitted revision.

A known
denial removes its complete provisional directory; an ambiguous child/transport
result remains pending. Stop calls hidden authenticated `agent-session broker
status` and permits exit only when the exact incarnation reports zero active and
zero uncertain operations. Local state is a retry cache and cannot satisfy
Stop. State never contains raw
arguments, output, prompt, capability content, or the provider call ID.

Read-only and unmatched post calls create no state. A private session lock
serializes capacity changes, successful terminal retry records carry a durable
monotonic completion sequence and compact to 64,
and the bridge refuses a new admission once 128 operation directories remain;
active, uncertain, malformed, extra, or currently locked state is never evicted
to make room.

Built-in semantic-conflict and owner-liveness admission applies only to a
managed process. When `AGENT_SESSION_ID`, `AGENT_SESSION_RUNTIME_ID`,
`AGENT_SESSION_STATE_DIR`, `AGENT_SESSION_COORDINATION_MODE`,
`AGENT_SESSION_CAPABILITY_FILE`, and `AGENT_SESSION_CHECKPOINT_FILE` are all
absent, both capabilities return `allow` with `coordination-unmanaged` without
loading or classifying registry owners, and the coordination transaction is not
invoked. Presence of any of those selectors without a complete session ID and
runtime incarnation is not unmanaged; it retains the conservative managed
failure posture. Environment-only advisory or off hints never downgrade an
untrusted managed identity.

A matcher on any other product/event pair is rejected during policy validation.
A policy matcher is either
one literal token or an anchored alternation expression such as
`Write|Edit|NotebookEdit|MultiEdit|apply_patch`; each atom is compared to the
entire normalized matcher. The only expression operator is `|`. Empty atoms,
duplicates, more than 64 atoms, and regex/glob/grouping/escape constructs are
invalid. Setup renders the original validated expression as one provider group
matcher, preserving registration grouping and order; it does not explode the
expression into per-tool rules. Oversized input, invalid UTF-8/JSON,
unsupported events, duplicate object keys, or unknown normalized fields fail
before rule evaluation. When ordinary config cannot be loaded, an exact valid
recovery capability evaluates its signed rule manifest and preserves every
ungranted rule instead of producing a global allow.

Provider rendering maps one normalized aggregate result to the event-native
output algebra. `PreToolUse` transforms render `permissionDecision = "allow"`
with `updatedInput`; Claude `PermissionRequest` transforms use
`decision.updatedInput`; Claude `PostToolUse` transforms use
`updatedToolOutput`; `PermissionRequest` decisions render
`decision.behavior = "allow" | "deny"`; Claude elicitation blocks render
`action = "decline"`; and other blocking events use the provider's documented
event-appropriate decision shape. Provider payloads that cannot be normalized
safely produce no stdout, a concise stderr diagnostic, and exit `2` so the
provider applies its native blocking fallback. Runtime or service-format
failures use exit `1`; successful provider-native decisions use exit `0`.

`dispatch --format json` returns the normalized decision envelope and does not
return raw provider content. DSH ingress requires this service JSON form; the
runtime bundle maps `block` to a `tools/pre-execute` denial, delegates `allow`,
and fails closed on every malformed, truncated, signaled, exit-mismatched,
replayed, timed-out, oversized, or unsupported response.

## Native DSH workspace lease

See [DSH workspace leases and recovery](dsh-workspace-lease-v1.md#native-dsh-workspace-lease).

### Protocol v2: target-scoped repository authority

See [DSH workspace leases and recovery](dsh-workspace-lease-v1.md#protocol-v2-target-scoped-repository-authority).

## Native DSH workspace recovery

See [DSH workspace leases and recovery](dsh-workspace-lease-v1.md#native-dsh-workspace-recovery).

## Native DSH finish line

`agent-hook finish-line <command> --format json` reads exactly one strict JSON
object from standard input. Version 1 accepts only `product = "dsh"`; every
request carries bounded non-space ASCII `session_id` and `turn_id` values plus
an absolute, already-canonical, non-symlink `cwd`. That value must be the exact
Git top-level resolved independently from both the request and the running
process; nested directories, nested repositories, unrelated repositories, and
non-repositories fail closed. Input is capped at 64 KiB; duplicate keys and
unknown fields fail with exit `65`. Every successful response carries the same
opaque `correlation_id` for one product/repository/session binding; consumers
must reject lifecycle responses whose correlation changes.

Finish-line commands default to the versioned service JSON envelope; text is
available only through an explicit `--format text` selection. Recoverable exit
`69`/`75` errors include typed `retryable`, `next_action`, and bounded
`recovery` details.

The DSH plugin routes every foreground Bash request through a non-executing
`run` probe before any execution. It rejects background Bash before invoking
the finish-line service. This is an integration boundary: the public
finish-line request does not accept a caller assertion that a command was
foreground, background, completed, or successful.

The public CLI surface is exactly `open`, `begin`, `run`, `register`, `admit`,
`observe`, `verdict`, `stop`, and `status`. Hidden `quiesce` and `release`
commands exist only for DSH cancellation, failed-execution cleanup, and
authenticated disposed-session retirement. Both are deliberately absent from
public help and completion. The four acceptance commands use the exact wire,
ordering, provenance, verdict, privacy, recovery, and compaction contract in
[`finish-line-acceptance-v1.md`](finish-line-acceptance-v1.md).

The request and response contracts are in [DSH finish-line operations](dsh-finish-line-v1.md).

## Serialized policy and capability registry

Policy bundles are strict TOML. This is the canonical serialized v1 shape;
unknown keys at every level are errors. Products/events are arrays so the same
stable rule can cover overlapping provider ingress. Capability-specific keys
live in the inline `capability` table and are rejected for a different ID.

```toml
schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.07.20.1"

[[rules]]
id = "runtime.session-start-healthcheck"
products = ["codex", "claude"]
events = ["SessionStart"]
priority = 100
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "runtime-kit.handler.v1", handler_id = "session-start-healthcheck" }
```

The stable built-in capability ID set for policy v1 is closed:

- `decision.allow.v1` (`reason_code`)
- `decision.warn.v1` (`reason_code`, `message`)
- `decision.block.v1` (`reason_code`, `message`)
- `decision.context.v1` (`reason_code`, `text`)
- `decision.transform.v1` (`reason_code`, `replacement`)
- `agent-session.activity.v1` (`reason_code`)
- `agent-session.owner-liveness.v1` (`reason_code`, optional
  `legacy_ttl_seconds`, maximum 900)
- `agent-session.semantic-conflict.v1` (`reason_code`)
- `agent-session.coordination.v1` (`reason_code`)
- `execution.read-only.v1` (`reason_code`, optional `fallback_handler_id`)
- `runtime-kit.handler.v1` (`handler_id` from the compiled v1 allowlist)
- `dsh.policy.v1` (`group` from the eleven implemented Task 3.2, nine
  implemented Task 3.3, and two implemented Task 3.4 `DshCapabilityGroup`
  values)

The separate `agent-hook.dsh-policy-capability-groups.v1` Rust/JSON schema
freezes the 23 deterministic groups selected by the DSH migration inventory.
The matching fixture records whether each group was delivered by finish-line
Task 2.3 or is owned by policy Tasks 3.2, 3.3, or 3.4. `dsh.policy.v1` makes
the Task 3.2 through Task 3.4 IDs executable only on their declared DSH
`PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `UserPromptSubmit`, or `Stop`
event; adding a fixture ID alone cannot activate it. Task 3.3 tool and lifecycle
rules may add bounded model
context, while neither task can transform tool arguments.

These evaluators use normalized exact
targets, bounded command parsing, trusted same-release nils companions, and
private session-bound lease state. The DSH parity verifier compares its ordered
nils-capability set to this fixture before either repository can claim inventory
parity.

Task 3.3 Bash privacy checks classify output redirection with quote-aware shell
syntax and resolve adjacent quoted target fragments before comparing paths.
Known writers expose literal destinations, destination-directory options are
joined with source basenames, dynamic or malformed destinations fail closed,
and an otherwise unknown executable with a protected-path operand is
indeterminate rather than read-only. Descriptor duplication and closing
(`N>&M`, `>&M`, `>&-`) are not file destinations, while any other `>&word`
names a file, as do zsh's clobbering `>!word` and `>&!word`.

An unknown
executable's operands make the write model unresolved only when the
quote-aware segment text can expand: `$` or backtick outside single quotes,
or an unquoted glob or brace character, `^`, `#`, a parenthesis, or a
word-leading tilde; quoted literals and a mid-word tilde such as `HEAD~1`
stay literal. A small closed set of non-writing tools
remains readable when no output redirection is present. MCP native writes and
partial edits scan maintained credential shapes, generic credential labels,
and structural sensitive JSON keys. Edit and str-replace old/new value pairs
also preserve the sensitive old-value context: replacement is allowed only for
removal or a strict `$NAME`/`${NAME}` environment reference whose name matches
`[A-Za-z_][A-Za-z0-9_]*`.

Fragments that cannot be parsed safely remain blocked
when they name a sensitive key. Startup memory is validated against the exact
companion schema and byte count, redacted across both shaped credentials and
generic `token`, `authorization`, or `private_key` labels, and rendered only as one escaped
`SHARED_AGENT_MEMORY_JSON` string so recalled content cannot terminate its
untrusted container.

For `block-unsafe-default-delivery`, the engine records an owner-private
projection of the canonical common Git directory identity and the unambiguous
cached remote HEAD before admitting Bash or native file mutation. The primary
checkout's current branch is not default-branch evidence and may differ from
the remote default. Multiple remotes must advertise the same default; missing,
contradictory, or drifted evidence is fail-closed, and native
write/edit/str-replace targets inside Git metadata are denied. The structured
`runtime_kit_governed_commit` tool is admissible only from its exact linked
worktree binding and has no repository-routing argument; outside any
repository there is no default branch to protect, so the seam admits the tool
and the runtime answers with its typed `no-repository` result.

Raw merge, pull,
cherry-pick, rebase, revert, am, reset, update-ref,
branch, push, and fetch shapes are classified against the resolved `git -C` or
semantic `--repo` target. `fetch --update-head-ok`, protected fetch
destinations, and stdin/server-driven ref-update plumbing are denied. A shell
builtin or assignment that can retarget cwd, exported variables, tracing hooks,
command lookup, or aliases before a later command makes the command
unclassifiable.

An unclassifiable, invalid, or nested-unresolved command fails
closed for every Tier A group and for a Tier B group whose subject appears in
the raw command (`git` for the commit, worktree, and default-delivery seams,
`gh`/`glab` for pull-request creation, `semantic-commit` for the body gate, an
agent-memory path for the project-memory seam, a machine-local path for the
portable-paths seam) or whose command field cannot be read at all; every other
Tier B or Tier C group explains the retry as context. Git command-consuming forms such as
rebase exec and submodule foreach, plus explicit transport-helper selection,
are rejected rather than partially parsed.

A command substitution, `$(...)` or backtick, whether unquoted, double-quoted,
in a `[`/`[[`/`test` operand, or in an unquoted-delimiter here-document body,
is classified as its own command ahead of the command it expands, which keeps
one dynamic placeholder argument. Shell comments and quoted-delimiter
here-document bodies are inert; a substitution or here-document that is nested
too deeply, unterminated, or not certainly read the way the shell reads it
stays unclassifiable. The `[` and `[[` test words and the `if`, `then`, `else`,
`elif`, `while`, `until`, `do`, and data-only `for NAME in` compound words are
classified command by command; `case`, subshells, and process substitution
stay unclassifiable.

The commit and default-delivery seams admit a Git builtin
and an installed `git-<name>` program (exec-path porcelain or a PATH
extension), which Git runs before any alias, unless the invocation retargets
PATH or GIT_EXEC_PATH lookup. `semantic-commit` help, `--dry-run`, and
`--validate-only` forms author nothing and are not default-branch delivery. The
`DSH_RUNTIME_KIT_WORKSPACE_LEASE_V2=1` selector defers the checkout guard
before any shell classification.

Exact abort/quit recovery and owned delivery CLIs remain admissible.
`semantic-commit --message-file` and repeated scalar message/repository options
are not accepted at this transcript boundary.

Trusted policy companions receive a cleared environment. The scope-lock
companion resolves a fixed trusted Git binary and disables repository
fsmonitor, hooks, pager, untracked-cache, external-diff, and textconv behavior
for every policy-time Git probe.

`execution.read-only.v1` evaluates only `PreToolUse` requests through the
same-release operation-effect verifier. Enforce mode admits only an exact,
fresh descriptor whose producer, cwd, target, argv, provider effect, and
OS-enforcement contract verify; missing, malformed, unsupported, mismatched,
or mutation evidence blocks. Shadow mode runs the same verification but records
only the redacted observation and cannot change admission.

An enforce rule may set `fallback_handler_id = "pre-edit-intent-gate"` only
when it is fail-closed and locked and each product/event/matcher binding has
exactly one later-priority enforce, fail-closed, locked
`runtime-kit.handler.v1` rule for that handler. A verified read-only operation
then bypasses only that paired rule; every other selected rule still evaluates.
Missing, malformed, unsupported, mismatched, or mutation evidence falls through
to the paired project-development handler instead of creating an aggregate
block. The paired handler's native result remains authoritative. No other
handler ID or shadow/downgradable pairing is valid.

`runtime-kit.handler.v1` is not arbitrary execution. The binary maps the
handler ID through a compiled allowlist to one exact runtime-kit-owned handler
basename and resolves it only below the active provider's owned hook directory.
The v1 allowlist is:

`agent-scope-lock-guard`, `block-agent-artifact-routing`,
`block-claude-coauthor-trailer`,
`block-direct-git-commit`, `block-direct-git-worktree`,
`block-direct-pr-create`, `block-direct-python`,
`block-project-memory-write`, `block-unsafe-default-delivery`,
`checkout-lease-guard`, `finish-line-record`, `forge-label-reminder`,
`mcp-secret-scan`, `memory-write-principle-reminder`,
`portable-paths-scan`, `pre-edit-intent-gate`,
`semantic-commit-body-gate`, `session-start-healthcheck`,
`skill-usage-reminder`, `stop-finish-line-gate`, `stop-pre-pr-reminder`,
`user-prompt-agent-docs`, and `user-prompt-agent-memory`.

The transitional DSH `pre-edit-intent-gate` binding remains recognized for policy
compatibility, but requests fail closed with a migration diagnostic. The
separate verified read-only fallback remains available. Policies must migrate
that DSH binding to the current runtime-kit validation contract before edits.

Handler resolution selects the compiled `.py` or `.sh` suffix, rejects
symlinks/non-regular files and owner/permission drift, passes the original
bounded provider JSON on standard input, and preserves the handler's provider
output/exit semantics. One dispatch starts at most 17 executable capabilities,
retains at most 512 KiB of aggregate child output, and shares a five-second
absolute deadline aligned with the per-handler timeout. Each child leads an
isolated process group; timeout or
direct-child exit terminates descendants before bounded pipe draining. The
policy cannot specify a path,
interpreter, argv, environment assignment, shell fragment, timeout, or digest.

Shadow mode never invokes it. Adding another handler requires a new nils-cli
release or a new versioned capability ID.

For Codex and Claude, `doctor` resolves every policy-referenced
`runtime-kit.handler.v1` and `agent-session.coordination.v1` handler before an
otherwise converged provider is reported healthy. Activation health and
immediate pre-execution validation share one trust predicate: the resolved
handler must be a regular file, executable by its effective-user owner, and
have no group/world write bits. Any failures are collected before returning a
typed `handler-unavailable` or `handler-untrusted` error; the diagnostic names
each affected handler without exposing its private resolved path.

`agent-session.coordination.v1` is a separate typed after-policy capability,
not another configurable handler ID. It resolves only the exact
`session-coordination-guard.py` consumer below the active provider's owned hook
directory, applies the same type/owner/mode/output protections, and passes the
original bounded provider payload on standard input. A dispatch can select at
most one such rule. The fixed consumer has a 55-second process bound under the
owned provider ingress's 60-second timeout so the #676 admit/complete/reconcile
transaction is not forced through the ordinary five-second handler budget. The
policy cannot select another command, path, arguments, environment assignment,
or timeout through this capability.

## Deterministic evaluation

See [Policy evaluation and degradation](evaluation-and-degradation-v1.md#deterministic-evaluation).

## Interaction lanes and degradation

See [Policy evaluation and degradation](evaluation-and-degradation-v1.md#interaction-lanes-and-degradation).

## Release compatibility

See [Policy evaluation and degradation](evaluation-and-degradation-v1.md#release-compatibility).

## Activity helper resolution

See [Provider setup, activity and recovery](provider-setup-recovery-v1.md#activity-helper-resolution).

## Setup ownership and rollback

See [Provider setup, activity and recovery](provider-setup-recovery-v1.md#setup-ownership-and-rollback).

## Governed recovery

See [Provider setup, activity and recovery](provider-setup-recovery-v1.md#governed-recovery).

## Coordination and writer liveness

The built-in `owner-liveness` capability lazily reads one bounded #676
coordination projection only when a selected enforced rule needs it. The
projection and heartbeat/fingerprint verifier live in `nils-common` and are
shared with the producer. Sidecar heartbeat evidence, not the registry's
projection timestamp, authenticates freshness. It validates ownership and permissions and
returns only public classifications. Active foreign writers and definite
semantic conflicts block. Stale clean ownership may be reclaimed atomically by
the owning coordination primitive. Dirty ownership requires governed adoption
or exact recovery. Orphaned and unknown evidence remain visible and
conservative. Compatibility state without broker evidence uses a documented bounded
compatibility TTL, never the historical eight-hour value as the primary
decision.

Crash, missing Stop, child exit, restart, stale heartbeat, key rotation,
concurrent contenders, and recreated-target evidence must not silently turn an
active/unknown owner into clear. Mailbox contents, raw session identity, host,
checkout path, and capability values never enter dispatch output or trace.

Semantic conflict is never accepted from a Codex or Claude payload field.
`agent-session.semantic-conflict.v1` derives its classification from the
private, owner/mode-validated #676 registry, the current managed principal's
ready/fresh broker incarnation, and active work-context claims. Exact worktree,
provider-reference, or overlapping scope evidence is definite; an incomplete
registry, missing authenticated current claim, stale peer universe, or only
repository-level uncertainty is `unknown`/`potential` and advisory. Only a definite authenticated conflict blocks. A forged provider field named
`semantic_conflict` is ignored and never upgrades or downgrades the derived
classification.

For a managed current principal, both coordination capabilities honor the
durable session mode. `advisory` downgrades coordination blocks to warnings,
`enforce` retains typed denial, and `off` does not participate. A downgrade is
accepted only when the private session record matches the current session and
runtime incarnation, its mode matches the fresh broker projection when one is
available, and any exported `AGENT_SESSION_COORDINATION_MODE` value agrees.
The environment value is a consistency hint, never downgrade authority by
itself. Missing, malformed, stale, or mismatched authority retains the
fail-closed/enforce behavior; a trusted durable advisory/off record may still
apply its non-denying failure posture when the registry is unavailable.

## Exit codes

- `0`: successful operation or provider allow/warn/context result.
- `1`: runtime failure or provider block.
- `64`: invalid CLI use.
- `65`: invalid config, policy, provider input, drift, or recovery data.
- `69`: required provider/setup resource or lock is temporarily unavailable.
- `75`: concurrency/lock contention suitable for bounded retry.

Claude PermissionDenied is observational progress and is registered by managed
Claude setup. It reports auto-mode denial, not a manual dialog denial. Permission
requests and tool progress retain no tool-name correlation metadata; same-turn
terminal evidence owns conservative permission-latch clearance.
