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
- `--require-idle` requires provider-reported `waiting` activity, no current turn,
  and no provider event ingestion queued behind the record lock. Missing,
  unknown, degraded, inferred, or console-only activity fails closed. Normal
  Claude completion hooks use `observed` confidence; both observed and
  authoritative provider-hook evidence are supported.
- `--require-empty-composer` requires an observable empty composer. A draft
  is refused without paste or Enter. `--allow-blocked` cannot override guards.
- `--idempotency-key` accepts 1–256 bytes and claims one delivery attempt for
  the stable managed session, across CLI retries and runtime replacements.

Admission runs inside the session record lock. The guard checks run again
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
pane. App-server sessions use the same terminal identity markers, provider
activity, exact composer capture and post-paste checks; this does not replace
them with a provider RPC. Unknown or invalid app-server profiles and other
providers or runtime kinds fail closed for these checks. Exact composer
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

A claim records metadata under a hashed filename in the stable session state;
it does not store the raw key or prompt. It has no expiry and is independent
of runtime generation. Every outcome after claim creation retains the claim,
including paste failure, guard refusal, timeout, crash, and an unverified or
stuck submission. A retry with that key refuses; it does not replay success.
This guarantees at most one delivery attempt, not eventual delivery. Inspect
uncertain outcomes before choosing a new key. Deleting the session state also
ends its deduplication history. Persistent storage must honor file and
directory synchronization for restart durability.

## Output and exit codes

Unguarded sends keep their existing output, submission retry behavior, and
exit codes. Guarded success adds only `guards: { "atomic": false }` to JSON;
text output retains the existing rendering. No raw key or composer text is
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
| `send-submit-stuck` | Text remains after the single guarded Enter; the claim is retained. |

Observation refusals include the failed `guard` and `stage` (`admission` or
`submit`), `atomic: false`, and whether paste preceded the check. Existing
session/runtime and I/O failure reasons still apply. A nonzero exit after
claiming must never be treated as permission to resend the same delivery.
