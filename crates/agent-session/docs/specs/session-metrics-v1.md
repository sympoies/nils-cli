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
| `resume` | Fixed unrecorded object described below |

Every metric has exactly `{ "value": unsigned_integer|null,
"unknown_reason": fixed_reason|null }`. A known value, including zero, has a
null reason. An unknown value is null with a non-null reason. A null creation
timestamp shares its age metric's reason. Ages use the same `observed_at`;
invalid or future timestamps yield `invalid_timestamp`, without clamping.

This slice always returns this resume object:

```json
{
  "kind": "unknown",
  "outcome": "unknown",
  "started_at": null,
  "completed_at": null,
  "deadline_at": null,
  "duration_ms": null,
  "unknown_reason": "not_recorded"
}
```

No existing runtime start, readiness, restart receipt or file timestamp supplies
resume latency. Runtime timing instrumentation is a separate slice.

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
after reading. Re-read the managed creation, runtime generation, current
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

Slice A emits only `not_bound`, `source_unavailable`, `identity_changed`,
`read_budget_exceeded`, `incomplete_source`, `unsupported_format`,
`ambiguous_completion_evidence`, `provider_creation_unavailable`,
`invalid_timestamp` and `not_recorded`. Existing source owners collapse missing,
unreadable, duplicate and truncated discovery into `source_unavailable`; the
endpoint does not invent a distinction those owners cannot prove. The accepted
schema also reserves `source_ambiguous` and the later timing reasons
`not_applicable`, `unsupported_runtime`, `launch_failed`, `provider_failed`,
`pending`, `timeout` and `clock_unavailable`.

Only the typed payload is serialized. No session object, provider record,
transcript text, source path, account/configuration metadata, arbitrary error
string or flattened extra reaches the metrics payload or diagnostics. Collection
writes no transcript evidence or timing receipt and makes no provider calls.

Counts describe retained content, not guaranteed lifetime totals. Large files
can remain unknown under these caps. There is no backfill or historical series;
consumers must retain their own timestamped samples and null reasons. No growth
or compaction threshold is justified by this endpoint alone.
