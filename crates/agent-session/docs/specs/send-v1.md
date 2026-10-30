# Send v1

`agent-session send <id>` delivers literal text, named keys, and single printable keys to one live
managed terminal. The JSON envelope remains `cli.agent-session.send.v1`.
[Session lifecycle operations](../runbooks/session-lifecycle.md) describes the
existing unguarded input and submission behavior.

## Printable picker keys

The CLI `--key` also accepts one printable Unicode character, such as `s` or
`1`. It sends a typed literal key rather than bracketed-paste text. Named keys
retain their existing meaning and completion candidates. Empty values,
multiple characters, control characters, and unknown multi-character key names
are rejected. The HTTP/WebSocket named-key contract is unchanged.

Here, printable means a single non-control Unicode character. Unicode format
characters, including U+200B and U+202E, are accepted and typed literally.

A send containing any printable key always enables `--require-idle` for the
whole request, including earlier named keys. It checks runtime identity,
provider activity and pending ingress before delivery and again before each key.
Busy, blocked, unknown, pending, unsupported, and unobservable sessions refuse
without initial input. `--allow-blocked` cannot override this requirement.
These are the same observational guards and race limits described below.

Printable keys use tmux literal mode with command separators escaped. The
success receipt includes the supplied character in `data.keys`; it confirms
transport, not a provider setting change. Inspect the provider before and after
with `glance` to verify model and effort. No provider menu is inferred and no
configuration file is written by this operation.

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
identity and requested idle state, and presses Enter. If the exact paste remains,
one swallowed Enter may be retried within a fifteen-second terminal observation
budget, with all guards rechecked before the retry. Native continuation keeps
its existing four-second budget. After an accepted Enter, verified absence of
that exact terminal runtime also confirms `submission.outcome: "submitted"`
(for example, `/exit`); no further Enter or cleanup is attempted. An unreadable
pane or unknown runtime status alone does not confirm submission. If a turn
starts or the composer changes after paste, it refuses without Enter. It
then attempts to remove only the text it pasted and verifies the result.

An exact paste can be removed; with Claude, a provable countable-cell paste
prefix can also be removed when admission required an empty composer and an earlier
post-paste observation saw exactly the pasted text before the conflicting edit.
Unreadable input is preserved. Stuck and unverified submissions use the same
cleanup path as post-paste guard failures. Post-paste refusal is distinct from admission refusal: it carries
`pasted: true` and `cleared`. When `cleared` is `false` the pasted text may
remain in the input, and the outcome stays unknown to callers.

## Observation limits

These terminal checks report `data.guards.atomic: false` on guarded success.
Guard refusal details also carry `atomic: false`. They are submission-time
observations, not an atomic compare-and-submit operation. A human keystroke or
provider turn can still race the interval between the final observation and
paste, Enter or cleanup keys. Cleanup cannot exclude simultaneous human edits
between its observations and keys. A native atomic provider operation is tracked separately in
[#2374](https://github.com/sympoies/nils-cli/issues/2374).

Idle and composer observations support Claude Code and Codex tmux runtimes,
including supported Codex `codex_app_server` profiles with their managed tmux
pane. App-server idle is native; composer capture remains a separate signal.
Unknown or invalid app-server profiles and other providers or runtime kinds
fail closed for these checks. Exact composer
verification supports an unambiguous, single visible input line.

With Claude and Codex,
a known single-line paste may also wrap across visible rows after
empty-composer admission. The entire region must uniquely match the paste;
cursor observations must prove its logical start, character count, and end
before Enter. Every character must be one cursor step one or two cells wide,
so wide (for example CJK) text counts steps in characters and endpoints in
cells. Padding and omitted word separators alone are insufficient proof.

Both providers' captures may omit final logical ASCII spaces as well as spaces
at word wraps; the cursor endpoint must still include every pasted cell.
Claude's padding normalization does not remove those cells from the known
paste's cursor proof. Each proof uses the active provider's composer region,
including when another provider's prompt appears in scrollback. Both providers
verify the actual logical end: Right then Left must return to N-1, then Right
must restore N. An invisible owner-appended blank cannot be admitted as part of
the paste's expected prefix.
Changed drafts, uncertain navigation, and styled wrapped suggestions refuse.
Dialogs, unknown layouts, other wrapped/multiline input, opaque long-paste summaries, and
unstyled placeholder text such as `Ask Codex to do anything` are unobservable.

Claude's composer is the box between its last horizontal border and the one
above the input row. A top border that carries a session title keeps only the
dashes the title leaves, so a long title can leave none: such a border starts
with the title and ends in a lone dash after a space. It opens the box only
when no full border already does, so rows of a draft never open it.

Guard captures use `tmux capture-pane -p -e -N -T` to retain occupied spaces
and exclude unused terminal cells.

Native placeholders and dynamic suggestions in either supported harness count
as empty only when every non-space input character carries dim styling (SGR 2),
the input neither begins nor ends with whitespace, and at least one character
is present. Claude styles each suggestion word on its own, so unstyled ASCII
spaces between dim words are part of the suggestion. Claude writes spaces after
its input up to the pane width in every capture, so unstyled trailing ASCII
spaces are renderer padding and not input; typed trailing spaces cannot be told
apart from it. Plain or partially dim suggestions remain drafts; fixed startup
hints without full dim proof are unobservable. Other whitespace drafts,
queued-message hints and opaque paste summaries never count as empty. Unsupported styling sequences fail closed. Grey color alone is not
treated as proof; a provider layout without distinguishable dim styling remains
unsupported. Placeholder content alone is never inferred empty.

For both supported providers, well-formed OSC-8 hyperlink metadata terminated
by BEL or ST is accepted without changing visible caption text or SGR state.
Each metadata
payload is bounded to 4096 UTF-8 bytes. Other OSC commands, malformed escapes
and screen-mutating controls remain unobservable. Extra composer rows are not
collapsed into empty input. Codex whitespace-only drafts remain nonempty;
hyperlink metadata never substitutes for dim proof or hides visible caption
text.

A guarded text-and-key send drops trailing line breaks before it pastes, since a
submitted prompt ignores them and a pasted break would add a composer row that
cannot be read back exactly. A line break anywhere else is refused before paste
with `send-text-multiline`.

The guarded post-paste check uses the full visible text, rather than the
unguarded submission classifier's prefix matching, ignoring only Claude's
renderer padding. A hidden draft cannot be discarded to make this check pass.

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
The authenticated conversation v2 route separately sends owner slash commands
through guarded TUI input; see [owner command admission](serve-prompt-v1.md).
This does not change the CLI native `/compact` profile or generic input authority.

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

### Native Claude notices

Only the internal notice-envelope route can use the managed Claude module.
An ordinary `send`, including one whose text resembles a notice, retains its
existing behavior. Gate-enabled fresh launches/resumes on Claude Code
`2.1.296` mount the module; other versions and existing runtimes without it keep
the unsupported notice admission boundary. There is no new flag, compatibility
mode, runtime injection or terminal fallback.

The native notice input is the complete LF-only UTF-8 envelope, at most 16 KiB.
Its spaces and final LF are never trimmed. Identity, idle, pending-ingress,
mounted-empty and used-key checks precede native authorization. A fresh native
handshake is required before durable key/operation consumption, and consumption
is synchronized before text release. The record lock is released before native
API entry so required hooks can run; no record writes occur while it is
released. Confirmation reacquires and checks identity in the same original
Claude send budget, without renewing it.

Success requires the exact returned input and one exact newly appended provider
row bound to that operation. It reports `native: { "transport": "claude_mod",
"confirmation": "notice_row_observed" }`, `submission.outcome: "submitted"`,
`submission.enter_presses: 0` and `guards.atomic: false`. No tmux input buffer,
paste, Enter, draft filling or clearing is used. A concurrent human edit remains
the human's input, even if it makes the notice unconfirmed.

Every postclaim drop, mismatch, unknown result, identity change or timeout keeps
the claim and returns `notice-native-unconfirmed` with unknown submission. It
cannot become the safe `notice-transport-unsupported` admission receipt, and no
error triggers replay or plain fallback. Expiry bounds local authorization and
confirmation; it does not cancel an already authorized remote provider call or
promise that it cannot complete later. See [session notices](session-notices-v1.md)
for the initial persistent metadata bound and consumer rollout requirements.

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
| `send-text-multiline` | Guarded text with a line break other than a trailing one; refused before paste. |
| `send-guard-observation-timeout` | The bounded guard observation budget expired. |
| `send-guard-unsupported` | The provider/runtime cannot perform a requested observation. |
| `send-idempotency-key-used` | This stable session already contains the delivery claim. |
| `send-idempotency-persist-failed` | The claim could not be durably written; no input follows. |
| `send-invalid-idempotency-key` | The key is outside the documented length bound. |
| `send-native-command-unsupported` | The native continuation profile cannot execute this slash command; no input follows. |
| `send-native-unconfirmed` | Native dispatch or subsequent exact-input confirmation is uncertain; the key is retained. |
| `send-submit-stuck` | Text remains after at most two guarded Enter presses; the claim is retained. |
| `send-submit-unconfirmed` | Pane observations cannot prove submission; the claim is retained. |

Observation refusals include the failed `guard` and `stage` (`admission`,
`claimed` for the recheck right after the idempotency claim, or `submit`),
`atomic: false`, and whether paste preceded the check. Existing
session/runtime and I/O failure reasons still apply. A nonzero exit after
claiming must never be treated as permission to resend the same delivery.

A failure while the guarded send checks or presses its Enter adds
`pasted: true`, `cleared` and `remaining_input` to its details. This includes
`send-submit-stuck` and `send-submit-unconfirmed`, which attempt the same
identity-fenced cleanup. A failure to write the session record after Enter
does not include these cleanup details.
`cleared: true` means a fresh observation verified the expected restoration:
empty input, or the preserved suffix after removing this send's prefix.
`cleared: true` does not mean the composer is empty: consumers must read
`remaining_input`. `false` means restoration was not verified; the error message explicitly says
whether the paste remains or the input is uncertain. `cleared` concerns the
owned paste, so an identical copy typed by the user can remain as their draft.

During the existing paste-settlement interval, cleanup records whether an
exact-paste observation preceded the conflict. Without that observation, a
draft typed before the paste can share its prefix; mixed input is left intact
and ownership is reported as unverified. With that observation, prefix recovery
treats later input as an appended suffix. This remains an observational
assumption; simultaneous edits can still race the observations and keys.

Cleanup uses the known paste to match wrapped Claude rows, considering written
row-end padding and separator spaces omitted by word wrapping. A unique match
is required; arbitrary unknown rows are never reconstructed. Submission
admission requires an exact single row or the cursor-proven wrapped paste
described above. Exact single-row matches retain
Backspace-only cleanup, including the existing trailing-space comparison.

For Claude wrapped or proven-prefix input of one- or two-cell characters,
cleanup moves to the logical line start, verifies the first composer row,
moves right by exactly the paste's character count, and verifies the visual
endpoint, so a guessed wrap separator cannot consume a draft character. Input
and runtime identity are rechecked before a bounded repeated-Backspace command removes that prefix.
Navigation and deletion are allowed to settle within the cleanup budget;
only a fresh post-deletion observation can establish restoration.

Mixed input without proof of an empty admitted composer, mixed drafts with
zero-width or control characters, blank continuation rows, dialogs, paste
placeholders, unexpected cursor positions and unreadable captures are
preserved and reported. Cleanup batches only the three identity variables it needs and pairs each pane/cursor read in
one tmux invocation, reducing subprocess round trips within the same budget.
An exact match on the existing Codex or non-ASCII path retains its
repeated-Backspace cleanup.

`remaining_input` classifies the last cleanup observation as `empty`,
`pasted_text`, `different_text`, or `unobservable`.
`paste_observed_before_conflict` states whether the earlier exact-paste read
succeeded; it does not claim atomic ownership. When observable,
`remaining_characters` counts the reconstructed input's Unicode scalar values
and `pasted_text_present` reports whether it contains the complete pasted text
as a substring, including any identical user-authored copy. These are
observations, not atomic guarantees; no composer content is returned.

Native refusal details always report `pasted: false`. `dispatched` indicates
that a native mutation was attempted; `acknowledged` records a valid provider
acceptance response. Before dispatch, `submitted` is `false`; after an
acknowledged turn it is `true` even if subsequent input confirmation fails;
without an acknowledgement it is `null` (unknown). None of these outcomes
permits reuse of the claimed key. Existing account and input-gate refusal
codes also apply.
