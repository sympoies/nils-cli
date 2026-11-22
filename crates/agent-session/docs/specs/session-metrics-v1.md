# Session metrics v1

## Endpoint and payload

`GET /sessions/{id}/metrics/v1` requires the daemon bearer token and fails
closed when none is configured. It resolves one managed session, collects in
one blocking task, and returns the ordinary envelope with `data.machine` and
`data.metrics`. A missing managed session uses the ordinary lookup error.
There is no CLI selector, subscription, federation operation or new listener.

`metrics` contains exactly these fields:

| Field | Type and meaning |
| --- | --- |
| `schema_version` | `"agent-session.session-metrics.v1"` |
| `observed_at` | RFC3339 UTC collector sample time |
| `provider` | `claude`, `codex`, `dsh` or `unknown` |
| `transcript_scope` | `"selected_primary_file"` |
| `compaction_scope` | `"retained_transcript"` |
| `transcript_bytes` | Metric: retained primary file length in bytes, including an unfinished suffix |
| `transcript_lines` | Metric: total LF bytes; CRLF counts once, escaped newlines do not count, unfinished suffix excluded |
| `completed_compactions` | Metric: distinct supported completion boundaries retained in that file |
| `managed_age_seconds` | Metric: whole elapsed seconds since managed creation |
| `conversation_age_seconds` | Metric: whole elapsed seconds since authoritative provider creation |
| `managed_created_at` | Normalized RFC3339 UTC timestamp or null |
| `conversation_created_at` | Normalized RFC3339 UTC timestamp or null |
| `resume` | Current runtime launch and restore observation described below |

Every metric has exactly `{ "value": unsigned_integer|null,
"unknown_reason": fixed_reason|null }`. A known value, including zero, has a
null reason. An unknown value is null with a non-null reason. A null creation
timestamp shares its age metric's reason. Ages use the same `observed_at`;
invalid or future timestamps yield `invalid_timestamp`, without clamping.

## Restore timing

The `resume` object always contains `kind`, `outcome`, `started_at`,
`completed_at`, `deadline_at`, `duration_ms` and `unknown_reason`. Timestamps
are RFC3339 UTC or null. Duration is an unsigned number of whole milliseconds
or null. Only a successful restore has a known duration and a null reason.

| Kind | Outcome | Unknown reason |
| --- | --- | --- |
| `initial` | `not_applicable` | `not_applicable` |
| `initial` or `resume` | `failed` | `launch_failed` if the gate write failed and the runtime record was retained |
| `resume` | `pending` | `pending` |
| `resume` | `succeeded` | null |
| `resume` | `failed` | `provider_failed` |
| `resume` | `timed_out` | `timeout` |
| `resume` | `unknown` | `unsupported_runtime` or `clock_unavailable` |
| `unknown` | `unknown` | `not_recorded` or `identity_changed` |

The canonical launch paths distinguish an initial launch from restoring an
existing provider conversation. A successful `start --resume` import is a
resume even at generation one. Recovery of a failed fresh launch is initial.
An already-running no-op and admission refusal leave timing unchanged.

Timing starts at the admitted launch-gate release attempt after broker
readiness. The interval includes persisting the bounded start receipt,
releasing the gate, gate polling, and provider startup. It excludes admission,
account preparation and shutdown of a previous runtime. The end depends on
the provider.

- Claude uses ingress of the first primary `SessionStart` with `source=resume`
  for the exact current runtime and provider conversation. Subagents and
  `startup`, `clear` and `compact` events cannot complete the observation.
- Managed Codex uses upstream receipt of the matching response to the initial
  TUI `thread/resume`. The request and returned thread must match the bound
  conversation. The result represents provider restoration. UI rendering and
  input readiness require separate observations. Control connections and raw
  Codex runtimes do not supply this signal.

A private receipt is bound to the managed creation, runtime generation, and
durable provider configuration and profile. Reads cap receipts at 4 KiB with
one overflow byte. Resume
receipts also bind the current provider conversation. A runtime replacement or
rebind invalidates the previous measurement. Each eligible runtime has a
60-second observation window. Late and duplicate events cannot update a
settled observation. A correlated Codex RPC error or an authenticated provider
runtime failure before restoration yields `provider_failed`. The daemon
projects timeouts on read. A successful duration uses the OS monotonic clock
on one boot. Wall clock timestamps label the events and may move when the
system clock is adjusted. Absent receipts, including pre-instrumentation
runtimes, yield `not_recorded`.

Timing adds no transcript reads, provider requests, background timer, or
history reconstruction. The provider event and receipt writes are best effort.
Startup projections, restart receipts and file timestamps never supply a
resume duration. Released event availability and timing semantics for both
providers are verified through separately scheduled native acceptance.

## Sources and identity

Use the existing provider-source owners for exact provider ID, effective
configuration root and profile selection. No newest-file selection or union of
roots is permitted. Discovery/header reads keep their existing independent
bounds (normally 5,000 entries and a cooperative 250 ms discovery slice;
Codex depth six). Source discovery can read bounded headers; it is not stat-only.

Claude uses one primary projects file with matching `sessionId`/`session_id`,
non-sidechain and a nonempty `cwd`, through the existing bounded header reader
(up to 64 leading lines of at most 1 MiB each). Codex uses a leading
`session_meta` with exact ID, `cwd`, parseable timestamp and scalar `cli` or
`vscode` source through the existing 64 KiB header reader. Exec/subagent
sources are excluded. Incomplete discovery cannot establish uniqueness.

Reject symlinks and nonregular files; compare the opened descriptor and path,
and compare file identity, size and modification/change timestamps before and
after reading. Codex header facts use that verified descriptor through the shared
provider header reader, then the descriptor is rewound for the complete count pass.
The descriptor supplies both header-derived age facts and counts.

Re-read the managed creation, runtime generation, current
conversation, configuration and profile binding before returning. An unresolved
native transition produces unknown and a GET never repairs its binding.
The native observation receipt read is capped at 16 KiB.
Changed identity invalidates the affected facts; no request retries or cache
can return stale measurements.

Codex provider creation comes from the existing header reader: payload timestamp
takes precedence over the top-level timestamp. Claude has no established
creation field, so conversation creation/age stay null with
`provider_creation_unavailable`. Managed creation never substitutes for it.

## Complete counting and cost

Only this explicit endpoint invokes metric counting. List, board and show add
zero metric content reads or fields; existing shared query discovery is unchanged.
Lines and compactions share one complete forward pass over the selected file:

- At most 8 MiB of counting content, in chunks of at most 64 KiB.
- A cooperative 250 ms counting deadline, separate from discovery/header work.
- At most 1 MiB buffered per parsed record; unknown JSON content is discarded.
- At most 4,096 distinct completion identities, each at most 256 bytes.

These are fixed implementation constants, with no new configuration knobs.
The deadline is checked between chunks; it cannot interrupt a blocking filesystem
call. This is not a hard whole-request I/O timeout. Over-cap or incomplete
scans never return partial totals. Independently established bytes/ages can
remain known when counts exceed the budget. Malformed JSON, an oversized
record or an unfinished suffix can invalidate compaction completeness while
leaving a complete physical-line count known. Blank terminated lines count.

Claude counts primary `system/compact_boundary` records, deduplicated by UUID.
Commands, echoes, summaries and started notices do not count. Missing boundary
identity is ambiguous. Codex counts canonical `compacted` records; accompanying
`event_msg` completion notices do not add counts. Stable IDs deduplicate repeated
canonical records. Repeated canonical records without distinguishable identities
are ambiguous; completion-notice-only forms are unsupported. No text comparison
is used to infer provider completion. These semantics describe supported fixture
shapes; native marker and retention acceptance is separately tester-scheduled.

## Unknown reasons and privacy

File and age metrics use `not_bound`, `source_unavailable`, `identity_changed`,
`read_budget_exceeded`, `incomplete_source`, `unsupported_format`,
`ambiguous_completion_evidence`, `provider_creation_unavailable` and
`invalid_timestamp`. Existing source owners collapse missing, unreadable,
duplicate and truncated discovery into `source_unavailable`. The accepted
schema reserves `source_ambiguous` for a source owner that can prove the
distinction.

Only the typed payload is serialized. No session object, provider record,
transcript text, source path, account/configuration metadata, arbitrary error
string or flattened extra reaches the metrics payload or diagnostics. The
timing receipt holds timestamps, monotonic ticks, an outcome and a private
binding digest. The endpoint makes no provider calls.

Counts describe retained content, not guaranteed lifetime totals. Large files
can remain unknown under these caps. There is no backfill or historical series;
consumers must retain their own timestamped samples and null reasons. No growth
or compaction threshold is justified by this endpoint alone.
