# State retirement inventory

This runbook defines the aggregate deployment-owner evidence requested by
[#3081](https://github.com/sympoies/nils-cli/issues/3081). It does not authorize
state changes or retire a runtime reader. Current producers and readers are
unchanged by the probe.

## Closed-ledger count

Run as the state owner, using the exact configured ledger location:

```bash
python3 crates/agent-session/scripts/closed-ledger-retirement-probe.py \
  --ledger "$AGENT_SESSION_STATE_DIR/board/closed-ledger.json"
```

Set `AGENT_SESSION_STATE_DIR` to the actual configured state directory first;
the probe intentionally does not discover directories or enumerate accounts.
Python 3.11 or newer and a Unix host are required. It opens only the supplied
file read-only, without a writer lock, network access or state repair. The
16 MiB read cap and owner/private-mode/single-link checks match the ledger
reader. A final-component symlink or non-regular file is refused. An open inode
provides one snapshot of the atomically replaced ledger, not a fleet-wide
transaction. Ordinary filesystem access-time accounting can still occur.

Do not use a board HTTP/CLI projection to count missing fields. The current
reader rejects a visible record missing `launch_env`; its ledger access can
also create, prune or replace the file.

The ledger must be UTF-8 JSON, as required by the runtime reader. The JSON output
contains fixed field names, observation epoch, status and
counts only. It never prints file paths, ledger identity, record values,
session/account identifiers or launch environment values.
Duplicate fields in the typed ledger or entry objects and nonstandard numeric
constants are invalid. Record metadata retains the runtime JSON map semantics;
it is not given a new field-uniqueness contract by this inventory.

| Output | Meaning |
| --- | --- |
| `status: observed` | A trusted current-schema ledger was read and counted. |
| `status: absent` | The file was absent; `counts` is null, not an observed zero. Exit 0. |
| `status: unavailable` | Access or file trust checks failed; `counts` is null. Exit 2. |
| `status: invalid` | Content, size, schema or sequence checks failed; `counts` is null. Exit 2. |
| `stored_entries` | All stored entries before retention. |
| `age_pruned_entries` | Entries older than seven days or without a readable close time. |
| `unreadable_closed_at_entries` | Subset without a canonical RFC 3339 close time the probe can parse. |
| `count_pruned_entries` | Excess entries after age pruning; keep the newest 256 in stored sequence order. |
| `visible_entries` | Entries remaining after both retention rules. |
| `visible_missing_launch_env` | Visible records with no `launch_env` member. Explicit null is not missing. |
| `visible_invalid_launch_env` | Visible records whose member is present but is not an object, including null. |
| `visible_launch_env_objects` | Visible records with object-shaped metadata; this does not validate its keys or values. |
| `visible_future_closed_at` | Visible close times after the observation epoch; the runtime retains these too. |

Exactly seven days old is retained. `--now-epoch INTEGER` pins the observation
time for synthetic acceptance or a coordinated comparison; omit it for a live
receipt. Unreadable time counts require investigation, because this small probe
does not implement every timestamp spelling accepted by the runtime parser.
They must not be used as proof that old data expired.

The acceptance test's seven-entry fixture at epoch `1800000000` contains two
visible missing members, one expired missing member, one unreadable time, one
null member, two object-shaped members and one future close time. Its expected
`counts` object is:

```json
{
  "stored_entries": 7,
  "age_pruned_entries": 2,
  "count_pruned_entries": 0,
  "unreadable_closed_at_entries": 1,
  "visible_entries": 5,
  "visible_missing_launch_env": 2,
  "visible_invalid_launch_env": 1,
  "visible_launch_env_objects": 2,
  "visible_future_closed_at": 1
}
```

Run synthetic acceptance without touching live state:

```bash
python3 -B -m unittest discover -s crates/agent-session/scripts -p 'test_*.py' -v
```

The suite also covers the inclusive age cutoff, timezone offsets, fractional
seconds, the 256-entry cutoff after age pruning, missing versus observed-empty,
invalid content/encoding/schema/sequence, duplicate typed fields, nonstandard
constants, non-regular/untrusted inputs, unchanged ledger
bytes and modification metadata, and a held writer lock.

For the first retirement slice, the owner receipt must include the observation
time, all configured state roots covered, supported active producer versions,
the last old-writer cutoff with evidence/confidence, and at least seven days
between that cutoff and observation. Require observed zero missing and invalid
members, zero unreadable/future times, or a separate explicit owner resolution.
An absent, unavailable, invalid or unobserved root needs its own disposition;
never silently convert it to zero. A probe result alone does not prove writer
cutoff or authorize retirement.

## State-root counts for slices 2–5

Run once per configured root as its owner:

```bash
python3 -B crates/agent-session/scripts/state-retirement-probe.py \
  --state-root "$AGENT_SESSION_STATE_DIR"
```

The output schema is `agent-session.state-retirement-probe.v1`. It contains
fixed keys, statuses, aggregate counts and `observed_at_epoch`; unknown schema
or provider strings enter fixed buckets. No persisted values become output keys. All leaves under observed `counts` are
non-negative integers, including buckets for singleton heartbeat flags.
The probe opens only `sessions/*/{session.json,auto-resume.json}` and the three
named coordination files below. It does not open prompts, logs, credentials or
provider settings. Files can contain unrelated sensitive fields; these are
parsed locally but never projected. There are no network calls, writer locks,
repairs, migrations or bytecode-cache writes. Filesystem access times can change.

Each store has independent `status` and `counts`. `observed` means that store
was scanned, not that the runtime would admit every record. `unobserved` means
missing, unreadable or untrusted input; `counts` is null. `invalid` means
malformed input or a scan limit; `counts` is null. The root-level status reports
whether the supplied directory was opened. A readable root can contain
unobserved stores. An empty existing sessions directory is an observed zero;
a missing directory is unobserved. Missing auto-resume sidecars are counted as
`absent_files` only after their session records were observed.

The probe refuses duplicate JSON members at every depth, NaN/Infinity,
non-finite numeric overflow, non-UTF-8 data, lone surrogate escapes and non-object
store documents. It accepts at most 64 MiB per file and 10,000 sessions-directory
entries. A failed session read invalidates or leaves unobserved the entire
session aggregate and its provider attribution; it never emits partial zeroes.
A failed auto-resume read affects that store alone. Files must be owned by the
caller, private (no group/other mode bits), regular and single-linked.
The supplied directory and opened child directories must be owned by the caller;
final-component and child symlinks are refused through descriptor-relative opens.
The caller selects trusted ancestors of the supplied path. Files are separate
inode snapshots, not a transaction across records or stores.

Exit 2 means an unobserved root or at least one invalid store. Exit 0 can still
contain unobserved stores or nonzero malformed-field buckets. Always inspect
every required store and bucket before claiming absence. `--now-epoch INTEGER`
pins the observation time for synthetic comparisons; it does not prune records.

In the table, paths are relative to the supplied root. `missing` means no member;
`null` means explicit JSON null. `present` validates only the stated field shape.
Other malformed field values enter `invalid`; container errors discard the
store aggregate. Unknown schema strings enter `other`, never a dynamic key.

| Slice / output under `stores` | Exact store and count definition |
| --- | --- |
| 2 / `auto_resume.recovery_terminals` | `sessions/*/auto-resume.json`: schema `agent-session.auto-resume.v1`, `enabled=false`, `state=terminal_failure`, `failure_reason=usage_window_not_exhausted`, string non-null `blocked_turn_id` and unsigned 64-bit non-null `blocked_revision`. Provider is `agent` from the sibling `session.json`, split into `claude`, `codex`, `other`. Zero revision and empty string follow the source predicate's non-null test. `other_records`, `invalid_records` and `absent_files` are separate. Current Codex terminals are not classified as retired Claude records. |
| 2 / `sessions.switch_intents` | `sessions/*/session.json`, flattened `codex_account_next`: schema `agent-session.codex-account-next.v1`; split `queued`, `applying`, `failed`, then `intent_id` missing/null/present/invalid. Present IDs match `[A-Za-z0-9-]{1,128}`. Absent intents and malformed intent objects/schema/state are separate counts. This counts the requested state/ID shape independently of account validity; it does not validate or allocate account/intent identity. |
| 2 / `sessions.title_pairs` | Same session records, `title` plus non-null `title_state`: exact stored equality against normalized current and v1.22 renderer outputs. `current` includes pairs accepted by both; `old_only` matches v1.22 and differs from current; `mismatch` matches neither. Missing/null state is `unstructured`; malformed state/title is `invalid`. The mirror follows `lib.rs` normalization, JS whitespace, reference boundaries, code-point truncation and activity budget. It never infers age from substrings. |
| 2 / `sessions.title_revision` | Same session records, raw `title_revision`: missing/null/present/invalid. Present means unsigned 64-bit integer, including zero, excluding booleans. No serde default is synthesized. |
| 2 / `sessions.session_incarnation` | Same session records, `runtime.launch_id`, as used by the session view. Missing/null runtime or absent launch member counts missing; explicit null launch ID counts null; nonempty string counts present; malformed runtime/launch value counts invalid. No incarnation is fabricated. |
| 3 / `registry.operations` | `coordination/registry.json`, `operations`: total; `state=expired` split by missing/null/integer/invalid `terminal_at_epoch`; known other states and malformed/unknown states counted separately. Claim expiry is not counted as operation expiry. |
| 3 / `registry.brokers` | Same registry, `brokers` values: total, runtime identity missing/null/object/non-object, and `binary_version` missing/null/string/non-string. An object identity is a shape, not authenticated runtime evidence. |
| 3 / `registry.completion_events`, `heartbeat` | Registry completion-event length; `coordination/broker-state.json` owner count, fixed v1 schema bucket, `completion_pending` true/false/invalid and `completion_registry` missing/null/object/non-object. Compare both files' separate snapshots; no registry stamp freshness or liveness claim is made. Missing heartbeat file remains unobserved. |
| 4 / `registry.schema`, `journal.schema` | `coordination/registry.json` schema buckets v1–v3; `coordination/federation-journal.json` schema buckets v1–v6. Missing/null/non-string/other are distinct. These are distributions, not a claim that the largest number is current. |
| 4 / `journal` envelope counters | Journal `remote_outbox` and `retained` lengths, outbox `state=queued` count, outbox envelope schema buckets session v1/v2 or service v2, category buckets and forwarding missing/null/object/non-object. Invalid envelopes are separate. Retained identities have no envelope bodies and are not included in envelope distributions. |
| 5 / `registry.mailbox` | Registry `messages`: total; `created_at_epoch_millis` missing/zero/positive/invalid (null, negative, non-integer or outside signed 64-bit range); `created_at_epoch` missing/null/signed 64-bit integer/invalid. Seconds and millis are parallel distributions, not a consistency or ordering check. |
| 5 / `sessions.retitle_markers` | Session `session_retitle_v2`: absent/invalid marker counts, fixed `agent-session.session-retitle-state.v2` schema bucket, receipt total, receipt `observation` missing/null/object/non-object. Empty receipt lists have zero observation counts. This does not validate observation readiness or inspect provider configuration. |

Session documents must be objects with schema `agent-session.session.v1` and a
nonempty string `agent`. These counters validate their named fields, not every
unrelated runtime field. Malformed-field, mismatch, unknown-schema and
unobserved counts require explicit owner disposition even when a target count
is zero. Do not equate object-shaped identity/stamp/observation with validity.

The slice-2 receipt must cover every configured sessions root and report the
Claude terminal, missing/null switch ID, old-only title, missing/null/invalid
revision and missing/null/invalid incarnation counts. It must also carry
supported-producer cutoff and client-contract evidence. These persisted counts
do not establish whether a supported client sends all title PATCH fences.

`client_contracts` remains explicitly unobserved. History/list cursors,
maintenance negotiation, transient PATCH requests and configured retitle command
providers have no identified persisted store in this probe. Registry replay,
idempotency and notification receipts, archived/history/closed-ledger title
pairs and report-comparison inputs are also outside this state-root scan; their
absence needs separate owner evidence where relevant. Run the slice-1 probe
separately for retained ledger metadata. No live state was inspected to build
these predicates.

Run the synthetic suite with the command above under **Closed-ledger count**.
It covers present, absent and malformed shapes, exact renderer boundary outputs,
strict JSON, file trust, aggregate privacy, preserved bytes/modification metadata
and a held writer lock.

## Remaining receipt definitions

The deployment owner collects these aggregates once per configured state root,
with an observation timestamp, source version, observed/unobserved status and
the last old-writer cutoff. No payloads, identities, digests, credentials or
private paths belong in the public receipt. Schema-name distributions are
public contract identifiers. An unknown shape is a separate count, not zero.
The source owners below define the predicates. The closed-ledger probe covers
slice 1; the state-root probe below covers the named persisted stores for
slices 2–5. Neither covers transient client requests.

| Slice | Requested counts or owner evidence | Source owner |
| --- | --- | --- |
| Usage recovery | Count `auto-resume.json` records with `enabled=false`, `state=terminal_failure`, `failure_reason=usage_window_not_exhausted` and non-null `blocked_turn_id` and `blocked_revision`; count other/invalid shapes separately. Do not print the blocked identifiers. | `src/auto_resume.rs`, usage-terminal predicate |
| Queued switch identity | Count session `codex_account_next` records by state and by missing/null versus present `intent_id`; report invalid shapes separately, without account or intent values. | `src/codex_account.rs`, `DurableNextAccount` |
| Rendered title pair | Count records accepted only by the predecessor title renderer, compared with the current renderer. Do not infer this from substrings or print titles. If a private predicate probe is unavailable, report unobserved. | `src/lib.rs`, structured-title normalization and `effective_session_title_state` |
| Operation terminal state | Count registry operations with `state=expired`, separately with/without `terminal_at_epoch`; include total operations and malformed-state counts. Claim expiry is a different current contract. | `src/coordination/mod.rs` |
| Broker/heartbeat evidence | Count broker records with missing/null runtime identity separately from malformed identity; count sessions carrying `binary_version`; count missing `broker-state.json`, missing/null `completion_registry`, completion queue entries and pending flag combinations. Unknown identity and unstamped crash recovery can be current safety behavior, so counts alone do not classify them as retired. | `src/coordination/broker.rs`, `heartbeat_state.rs` |
| Registry/journal/envelopes | Schema distributions for `coordination/registry.json` and `federation-journal.json`, queued/retained entry totals and envelope schema/category/forwarding-presence distributions. Include replay/idempotency and notification receipt shape counts without bodies, identity/digest values or keys. Compare actual selector output and active producers; do not designate the highest number current by assumption. | `src/coordination/{mod.rs,delegation,remote.rs,remote_journal_version.rs,notification_observation.rs}`; `nils-common/src/coordination_projection.rs` |
| Mailbox time | Count stored messages with missing, zero or positive `created_at_epoch_millis`, and compare presence of canonical seconds fields; do not print time-ordered message IDs or content. | `src/coordination/mailbox.rs` |
| History/title/maintenance clients | Owner inventory of supported clients consuming `prompt_preview`, issuing predecessor list cursors, omitting title revision fences or negotiating predecessor maintenance contracts. These may be transient requests, so a disk count cannot establish absence. Count persisted client cursor/schema forms only where the owner can identify their stores; otherwise report unobserved. | `src/provider_history.rs`, `src/maintenance.rs`, session PATCH owners |
| Retitle | Count configured `provider=command`, persisted `session_retitle_v2` marker/schema distributions and missing observation members; preserve distinct current unknown/readiness states. Do not print provider arguments, prompts, model output or account settings. | `src/retitle.rs` and projection owners |
| Cooldown | Owner inventory of active token-keyed producers and unknown-account cases; aggregate cooldown-file total and future retry-deadline count. Classifying account versus token ownership requires owner-local knowledge; do not infer it from opaque filenames or disclose account/token/digest values. | `claude-cli/src/prompt_segment/backoff.rs`, `backoff/account.rs` |
| Report comparison | Count retained report objects with `schema` present, with only the predecessor `schema_version` key, or with neither/invalid forms, including report envelopes. Do not print report contents. | `agent-workflow-primitives/src/repo_retro.rs`, `compare_prior_report` |

Notification rows already retired by #3345 are not new work in this inventory.
The deployment owner routes any required one-shot migration separately. Signed
records and evidence provenance remain immutable. Any proposed cross-host
window needs the exact adjacent release pair, first-disallowed release and hard
expiry; a local replay or daemon rollback does not supply that justification.
