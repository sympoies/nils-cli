# Lifecycle incident evidence

`agent-session` keeps a private, bounded lifecycle journal separate from pane
logs. Read it without a live runtime, broker, or serve daemon:

```sh
agent-session logs --lifecycle SESSION_ID --tail 120 --format json
```

The journal lives below the configured state directory at
`lifecycle/SESSION_ID/`. `current.jsonl` and `previous.jsonl` each hold at most
256 KiB; rotation replaces the oldest segment under a per-session lock. Records
remain available after session deletion. This is a rolling incident journal,
not an unlimited audit archive. The default read returns the latest 120 records,
with a maximum of 1,000. Files are owner-only; untrusted or linked journal paths
are refused. Journal write failures emit `lifecycle-journal-unavailable` without
replacing the original lifecycle result.

Each record contains a timestamp, operation, CLI/serve/controller caller, caller
session ID when available, executable version and path, target session,
incarnation and generation, and success or a typed error with its failing proof
step. Read `result.proof_step` to distinguish incarnation fences, fresh
heartbeats, runtime stopped proof, capability checks and operation quiescence.
The journal omits error details and replaces dynamic error messages with a fixed
summary because provider errors may contain private input. Static audited
refusal messages are preserved. It never records prompts, terminal output,
provider arguments, account names, or capability contents.

Start/run, provider-history import, resume, delete/archive, account switch,
broker adopt/reconcile and broker stop attempts are recorded. Authenticated
serve create/import/resume/account/delete requests record the final HTTP outcome,
including validation before engine entry. The HTTP error envelope preserves the
engine's original error code and message. Failed runtime creation returns an
error envelope instead of a successful stopped-session response; retained
session diagnostics remain available through the normal session read routes.

The launch wrapper reports observed exit status. Inventory and controller
probes also record a stopped runtime once per incarnation when the wrapper
cannot report. Such evidence says `runtime disappeared outside agent-session`
unless a successful agent-session stop is known. Unknown exit status, signal,
and actor stay null. A lost heartbeat records a broker degradation transition,
not an invented process exit. Absence of a lifecycle exit record does not prove
that a runtime remains alive: a stopped controller cannot observe its own loss.

During an incident, compare the refused operation's proof step with the current
`agent-session broker status --session SESSION_ID` and `agent-session diagnose`
output. A journal record is diagnostic evidence; it does not grant capability
or authorize a runtime replacement.
