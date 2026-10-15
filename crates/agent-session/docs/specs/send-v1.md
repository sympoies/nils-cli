# Send v1

`agent-session send <id>` delivers literal text and named keys to one live
managed terminal. The JSON envelope remains `cli.agent-session.send.v1`.
[Session lifecycle operations](../runbooks/session-lifecycle.md) describes the
existing unguarded input and submission behavior.

## Opt-in continuation guards

```bash
agent-session send <id> --text-stdin --key enter --format json \
  --expect-runtime <launch-id> --expect-provider-session <provider-session-id> \
  --require-idle --require-empty-composer --idempotency-key <key>
```

Each option is independent:

- `--expect-runtime` matches the persisted runtime launch ID. Guarded sends
  also verify the live terminal's session and runtime environment markers.
- `--expect-provider-session` matches the persisted provider resume session ID
  for the session's provider. An absent binding cannot match.
- `--require-idle` requires no provider event ingestion queued behind the record
  lock. Terminal runtimes require provider-reported `waiting` activity and no
  current turn; missing, unknown, degraded, inferred, or console-only activity
  fails closed. Normal Claude completion hooks use `observed` confidence;
  observed and authoritative provider-hook evidence are supported. Supported
  Codex app-server runtimes verify the exact attached, loaded thread is `idle`
  through the native protocol. Outstanding activity attention still refuses,
  including when `--allow-blocked` is present.
- `--require-empty-composer` requires an observable empty composer. A draft
  is refused without paste or Enter. `--allow-blocked` cannot override guards.
- `--idempotency-key` accepts 1–256 bytes and claims one delivery attempt for
  the stable managed session, across CLI retries and runtime replacements.

Admission runs inside the session record lock. The terminal path below applies
unless the native continuation profile described later is selected. The guard checks run again
between loading the private tmux buffer and pasting it. The durable key is
claimed and synced, including its parent directory, before any pane input.
A final admission check follows persistence. No refused admission pastes or
presses keys. A provider event waiting for the record lock advertises ingress;
short, nonblocking guard probes detect it without dropping that event.

For a guarded text-and-Enter submission, send waits for paste settlement and
then requires the composer to contain **exactly** its own text, rechecks
identity and requested idle state, and presses Enter at most once. If a turn
starts or the composer changes after paste, it refuses without Enter and
preserves the composer, including user additions. It never clears user input
or retries a swallowed Enter. Post-paste refusal is distinct from admission
refusal: the text may already be present in the composer.

## Observation limits

These terminal checks report `data.guards.atomic: false` on guarded success.
Guard refusal details also carry `atomic: false`. They are submission-time
observations, not an atomic compare-and-submit operation. A human keystroke or
provider turn can still race the interval between the final observation and
paste or Enter. A native atomic provider operation is tracked separately in
[#2374](https://github.com/sympoies/nils-cli/issues/2374).

Idle and composer observations support Claude Code and Codex tmux runtimes,
including supported Codex `codex_app_server` profiles with their managed tmux
pane. App-server idle is native; composer capture remains a separate signal.
Unknown or invalid app-server profiles and other providers or runtime kinds
fail closed for these checks. Exact composer
verification supports an unambiguous, single visible input line. Dialogs,
unknown layouts, wrapped/multiline input, opaque long-paste summaries, and
unstyled placeholder text such as `Ask Codex to do anything` are unobservable.

Guard captures use `tmux capture-pane -p -e -N -T` to retain occupied spaces
and exclude unused terminal cells.

The native Codex placeholder and
Claude `Try "…"` hint count as empty only when every hint character carries
dim styling (SGR 2). An unstyled or partially styled hint fails closed, as do
unsupported styling sequences. Grey color alone is not treated as proof.
A provider layout that exposes no distinguishable dim hint cannot support
guarded empty-composer sends. Placeholder content alone is never inferred empty.
For Codex only, well-formed OSC-8 hyperlink metadata terminated by BEL or ST
is accepted without changing visible caption text or SGR state. Each metadata
payload is bounded to 4096 UTF-8 bytes. Other OSC commands, malformed escapes
and screen-mutating controls remain unobservable. Extra composer rows are not
collapsed into empty input. Claude retains its existing parser and refuses
OSC-8 captures or indistinguishable empty/whitespace composer states.

The guarded post-paste check uses full visible text, preserving whitespace,
rather than the unguarded submission classifier's prefix matching. Occupied
padding or a hidden draft cannot be discarded to make this check pass.

## Native Codex continuation profile

A supported app-server text submission with exactly `--key enter` and both
`--require-idle` and `--require-empty-composer` uses native dispatch. Partial
flag selections retain their existing terminal transport; this is route
selection, never a fallback after failed native admission or delivery.

The native path preserves terminal runtime markers, durable provider identity,
pending-ingress and attention checks, and the independently observable empty
TUI composer. An idle provider thread does **not** prove the attached TUI has
no unsent draft. Unknown, whitespace-only or text drafts remain refused. No
new layout or styling is assumed observable merely because the runtime is
app-server bound.

The session-record lock covers admission and dispatch through the acknowledged
turn ID. It is released before observing input history, so synchronous provider
hooks can acquire it and allow the submitted input to become visible. All
record writes finish before release; confirmation never rewrites an older
record over provider updates. Before returning success, the sender reacquires
the record lock within the same budget and revalidates runtime and provider
identity. A replacement or unavailable identity leaves the receipt unconfirmed.

The existing nonblocking input/account mutation gate remains held through confirmation. A pending next-account intent still
fences the new input. The key is durably claimed before one native
mutation; admission is repeated after claiming. There is no tmux buffer,
paste, Enter, composer clearing, or delivery retry on this path, including
when confirmation fails. Concurrent human typing can still race observation;
`atomic: false` remains accurate.

Ordinary prompts use `turn/start`. Success requires an acknowledged turn ID
and observation of the exact input in that turn of the same thread, within
the existing four-second send budget. History reads are bounded by the
app-server WebSocket message cap; unavailable or oversized history cannot
confirm delivery. No execution success or eventual delivery is implied.

A payload whose trimmed content is exactly `/compact` uses
`thread/compact/start`, including a trailing stdin newline. Its success means
**compaction start was acknowledged**, not compaction completed. The caller
must observe confirmed completion before issuing a continuation. Other leading
slash commands refuse with `send-native-command-unsupported`; they are never
silently converted into model prompts or delivered through a terminal fallback.

Native success adds `native: { "transport": "codex_app_server",
"confirmation": "input_observed" | "compact_started" }` and reports
`submission.outcome: "submitted"`, `submission.enter_presses: 0`. `keys`
retains the requested logical keys; no physical Enter was pressed.

A claim records metadata under a hashed filename in the stable session state;
it does not store the raw key or prompt. It has no expiry and is independent
of runtime generation. Every outcome after claim creation retains the claim,
including paste failure, guard refusal, timeout, crash, and an unverified or
stuck submission. A retry with that key refuses; it does not replay success.
This guarantees at most one delivery attempt, not eventual delivery. Inspect
uncertain outcomes before choosing a new key. Deleting the session state also
ends its deduplication history. Persistent storage must honor file and
directory synchronization for restart durability.

## Opt-in native diagnostic trace

`AGENT_SESSION_GUARDED_TRACE_FILE` enables a private timing trace for a supported
app-server send with both idle/composer guards and exactly one logical Enter.
Without the variable, no trace file is created. It is diagnostic only: admission,
locks, the four-second budget, dispatch, deduplication, output and exit codes
retain their existing behavior. It never retries input or supplies a fallback.

The value must name a new absolute file under a canonical, existing directory
owned by the caller with no group/other permissions (for example, mode `0700`).
Symlink ancestors, a symlink target, existing files and unsafe parents are
refused for tracing. Creation uses mode `0600` and does not overwrite evidence.
Invalid paths or diagnostic write failures leave the send outcome unchanged;
a missing, empty or truncated trace is incomplete evidence.

Events are buffered in memory and written once after the send releases its
record/account locks. The JSONL schema is `agent-session.guarded-trace.v1`, with
at most 256 records; the last is `trace_truncated` if that bound is reached.
Every record has a fixed `stage` and monotonic `elapsed_us`, relative to trace
activation before record-lock acquisition. Record-lock wait/acquisition and
send-budget start delimit the admission timeline. File setup precedes this
clock. The record-lock wait is this send's wait, not a provider hook's wait.

RPC records use a trace-local numeric `request` shared by the request/response
pair, a fixed `method`, optional `include_turns`, request-time `remaining_us`,
and a fixed response `outcome`. Methods are `connect`, `initialize`,
`loaded_threads`, `thread_read`, `turn_start` and `compact_start`. Outcomes
include `ok`, `timeout`, `unavailable`, `rejected`, `deadline_expired` and
`cancelled` (an unfinished nested request dropped by its enclosing operation).
Response time means the correlated RPC response was received, not delivery
confirmation. A `turn_acknowledged` record classifies validity of the returned
turn identity. Input observations report `observed`, `not_observed` or `invalid`
and optional thread/turn/text equality booleans. Missing fields remain unknown;
no raw identity, input, composer, history, credentials, path or provider error
text is persisted. Observation results do not relax exact-input matching.

A diagnostic run uses the same guarded command and one fresh operation key:

```bash
# Configure these values for the scheduled disposable session.
# TRACE_DIR must be an existing canonical absolute directory owned by the caller.
chmod 700 "$TRACE_DIR"
AGENT_SESSION_GUARDED_TRACE_FILE="$TRACE_DIR/native-send.jsonl" \
  agent-session send "$SESSION_ID" --text-stdin --key enter \
  --expect-runtime "$RUNTIME_ID" --require-idle --require-empty-composer \
  --idempotency-key "$OPERATION_KEY" --format json < "$INPUT_FILE"
```

Retain the trace privately with the JSON send receipt and version binding.
Publish only reviewed classification/timing/equality metadata. An unconfirmed
receipt still retains its key; neither a missing trace nor later provider
completion permits another send. Hook dispatch and hook lock-wait timings are
not collected here; retain them as unavailable rather than inferring them from
RPC timing or a later completion. The separate read-only `agent-session diagnose`
bundle provides control-plane classifications, not an RPC or hook-wait trace.

## Output and exit codes

Unguarded sends keep their existing output, submission retry behavior, and
exit codes. Guarded success adds `guards: { "atomic": false }` to JSON;
the native continuation profile also adds the optional `native` receipt above.
Text output retains the existing rendering. No raw key or composer text is
included in refusal output. Use `--format json` to consume `error.code` and
`error.details` without parsing prose.

| Exit | Meaning |
| --- | --- |
| `0` | Input delivery completed; inspect `submission.outcome` where present. |
| `1` | Guard refusal, persistence failure, stopped runtime, or delivery failure. |
| `64` | CLI usage error, including an empty/oversized idempotency key. |
| `65` | Invalid session data or another existing data-contract refusal. |

| Reason (`error.code`) | Meaning |
| --- | --- |
| `send-identity-mismatch` | Expected or live runtime/provider identity differs. |
| `send-identity-unobservable` | Runtime identity or terminal markers are unavailable. |
| `send-session-not-idle` | A provider-reported phase or current turn is not idle. |
| `send-provider-event-pending` | Provider event ingestion is queued/in progress. |
| `send-idle-unobservable` | Provider-reported activity or ingress status cannot be observed. |
| `send-composer-not-empty` | A visible draft exists before paste. |
| `send-composer-unobservable` | The composer cannot be read exactly. |
| `send-composer-mismatch` | After paste, the composer differs from the submitted text. |
| `send-guard-observation-timeout` | The bounded guard observation budget expired. |
| `send-guard-unsupported` | The provider/runtime cannot perform a requested observation. |
| `send-idempotency-key-used` | This stable session already contains the delivery claim. |
| `send-idempotency-persist-failed` | The claim could not be durably written; no input follows. |
| `send-invalid-idempotency-key` | The key is outside the documented length bound. |
| `send-native-command-unsupported` | The native continuation profile cannot execute this slash command; no input follows. |
| `send-native-unconfirmed` | Native dispatch or subsequent exact-input confirmation is uncertain; the key is retained. |
| `send-submit-stuck` | Text remains after the single guarded Enter; the claim is retained. |

Observation refusals include the failed `guard` and `stage` (`admission` or
`submit`), `atomic: false`, and whether paste preceded the check. Existing
session/runtime and I/O failure reasons still apply. A nonzero exit after
claiming must never be treated as permission to resend the same delivery.

Native refusal details always report `pasted: false`. `dispatched` indicates
that a native mutation was attempted; `acknowledged` records a valid provider
acceptance response. Before dispatch, `submitted` is `false`; after an
acknowledged turn it is `true` even if subsequent input confirmation fails;
without an acknowledgement it is `null` (unknown). None of these outcomes
permits reuse of the claimed key. Existing account and input-gate refusal
codes also apply.
