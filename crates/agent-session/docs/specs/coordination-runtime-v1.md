# Coordination runtime and recovery v1

Part of [Session Coordination V1](session-coordination-v1.md). Its scope and shared rules also apply here.

## Managed launch and broker boundary

Start, run, resume, provider-import, and HTTP create follow one transaction:

1. reserve the session record and hold its lifecycle lock;
2. create the tmux pane in a held state that cannot exec the agent;
3. persist and read back the exact tmux/runtime identity;
4. start the runtime-owned heartbeat sidecar before the held gate;
5. create the private per-incarnation capability under the registry lock; the
   hidden sidecar command requires that credential as launch authority and
   waits at most 2 seconds for exact identity-bound readiness;
6. only then release the held pane to exec the agent.

Failure at any boundary revokes credentials, stops the broker, terminates only the exact held runtime,
and preserves bounded startup diagnostics. Launcher exit does not stop an established broker. Resume
creates a replacement incarnation and capability. Broker loss blocks new coordination operations. `broker
adopt` requires an unchanged, live, exactly matched runtime and never trusts a PID or pane name alone.
Recovery first persists a non-ready `recovering` state; the sidecar may heartbeat while fenced, but
readiness, operation reconciliation, and the idempotency receipt become visible only in one final
registry commit.

Runtime uncertainty moves the broker to `degraded` without releasing claims or operations; only positive
stopped-runtime evidence may revoke them. On Linux, that evidence includes a valid persisted
PID-namespace identity whose boot ID differs from the current boot: processes from that namespace cannot
survive the reboot. A missing namespace identity or a same-boot namespace mismatch remains unverified and
fails closed. On macOS, newly launched and exactly captured tmux runtime identities persist the canonical
kernel `kern.bootsessionuuid`.

The additive UUID is retained in broker runtime evidence but excluded from the existing identity digest
so a rollback preserves broker and lease comparisons. A valid different boot UUID is positive
stopped-runtime evidence even if numeric PID or tmux IDs have been reused. Same-boot or missing boot
evidence does not upgrade process-group absence into coordination authority. Older records retain their
conservative behavior; no migration invents a boot identity for an absent runtime. Natural target exit
immediately removes its incarnation-specific capability; a replacement uses a different path, so a stale
runtime can never read the new credential.

Delete also releases terminal coordination state before session removal is reported complete.

The optional HTTP server is not the heartbeat owner and is not required for
coordination after launch.

The sidecar re-checks its authorization on every beat through a read-only
registry observation that runs no maintenance and never rewrites the registry.
It exits only when that check proves revocation: a missing capability, or a
broker entry for another incarnation, generation, capability, or non-live
state. When the registry cannot be read, for example because the lock stays busy
past its timeout, the sidecar skips that beat and retries on the next one
instead of exiting.

Full registry maintenance prunes a `stopped`, capability-less broker entry once
its stop is at least 24 hours old, no active claim or operation names its
incarnation, and its session record no longer exists.

Broker recovery is an authenticated owner mutation, not an operator-only
repair. The canonical HTTP routes are
`POST /sessions/{id}/broker/{adopt,reconcile}/v2`; they require both the server
bearer and `X-Agent-Session-Capability` for the exact persisted session
incarnation. The proof remains in the request body. The `/v1` POST routes are
retained only as transition aliases for the same strong authorization contract;
starting with 1.25.11, bearer-only callers fail with
`coordination-unauthorized` before registry mutation. Callers migrate by
supplying the capability header and selecting `/v2`; a copied capability from
another session or a replaced incarnation is rejected. This security boundary
does not require a fresh heartbeat, because stale or absent heartbeat evidence
is the state recovery repairs.

### Operator recovery after an external runtime stop

If a runtime was stopped outside `agent-session` and ordinary `resume` refuses
its retained broker, use the local owner-only `broker retire-stopped` command.
This route retires the old broker; it does not launch a runtime or change the
provider conversation. Ordinary broker liveness proofs remain conservative.
There is no HTTP route and no runtime capability can authorize this operation.

1. Inspect `agent-session show SESSION --format json` and
   `agent-session broker status --session SESSION --format json`. Copy the exact
   persisted `runtime.launch_id` and `runtime.generation`; never guess a tuple.
2. Preview with those selectors and one stable request key:

   ```sh
   agent-session broker retire-stopped --session SESSION \
     --incarnation LAUNCH_ID --generation GENERATION \
     --idempotency-key REQUEST_KEY --dry-run --format json
   ```

3. Read every `proofs` row. Apply only when `eligible` is true, using the same
   command with `--apply` instead of `--dry-run`. Then run
   `agent-session resume SESSION --format json` to launch the next generation
   of the retained conversation.

The stable result schema is `agent-session.broker-retirement.v1`. Preview is
the default, holds the same session lifecycle lock as resume followed by an
observational coordination lock, and does not run registry maintenance or
change registry/session bytes. It reports every proof as `{step, passed}`,
including the selected `stale_after_seconds`. `--stale-after SECONDS` explicitly
sets the heartbeat threshold (default 600, allowed 60–86400); elapsed time alone
never proves a runtime stopped. Preview attempts are recorded in the private
lifecycle journal.

Apply requires exact matching session and broker incarnation/generation and
persisted runtime identity, current boot evidence, a stale registry heartbeat
and stale or absent trusted heartbeat file, absent exact managed tmux name and
recorded numeric target, absent pane process, and positive absence of every recorded process boundary. On Linux this
requires the same PID namespace and complete process inspection; on macOS it
requires the same kernel boot UUID, signal-zero absence and a bounded complete
process-group snapshot. Missing, malformed, permission-denied, live, or reused
evidence refuses retirement. Prior runtime boundaries must also be absent.
Unresolved operations (including expired `reconcile_pending` leases) and an
in-flight notification submission refuse apply; resolve them through their
owning workflow.

Apply repeats kernel, tmux and heartbeat probes immediately before saving.
One atomic registry replacement sets the broker to `stopped`, revokes its
authentication digest, releases its active claims, removes advisory
state, and stores the idempotent receipt. The session and provider-resume
identity remain intact. Private capability, heartbeat and checkpoint removal
then completes under both locks. `coordination-retirement-cleanup-pending`
means revocation is committed: replay exactly the same selectors, threshold and
request key to finish cleanup. A successful replay returns the original receipt
without a second retirement. Receipts use the existing bounded 24-hour registry
retention; a different request with a reused key or a replacement generation
fails closed. Keep the returned receipt as operator evidence.

### Operation diagnostics and reconciliation

Every Console start success and operational failure includes a canonical time-bearing UUIDv7
`operation_id`. Supply `--operation-id UUID` before submitting when the caller
must recover even if its own CLI response is lost. The CLI and relay daemon use
the same identifier; the daemon also sends it as the
`X-Agent-Session-Operation-Id` HTTP header to the aggregator. This header is
correlation metadata, not an upstream idempotency guarantee.

`diagnostics.coordination_lock_wait_ms` measures time spent acquiring
coordination file locks in the CLI and relay daemon, including timed-out
acquisitions. `diagnostics.command_elapsed_ms` measures the CLI command from
request preparation through its result, including network and launch latency.
Both are integer milliseconds from monotonic clocks. Lock wait is a component
of elapsed time, not an additional duration. They do not claim to measure locks
inside the aggregator or target daemon. Text errors include the identifier,
these timings, and the reconciliation command in a hint.

Use `agent-session start --via-console --operation-id UUID --reconcile --format
json` to inspect a prior operation. This performs no create request. The
private daemon receipt is bound to the authenticated caller incarnation and
request fingerprint. Repeating the original submission with the same UUID
also reconciles; changed arguments return `console-start-operation-conflict`.
Older callers that omit the UUID still receive a generated identifier.

- `reconciliation.state: created` reports the recorded existing session ID,
  including a child created before registration or response delivery failed.
  It does not launch, resume, or reattach the child, and does not assert the
  child is still running. `safe_to_retry` remains false.
- `start_failed` records a definite provider startup failure, retaining the
  addressable failed attempt under `session.id`. `failure` includes code
  `start-failed`, a recognized startup `reason`, nullable numeric
  `provider_exit_code`, `cleanup`, `retryable`, `safe_to_retry`, `next_action`,
  and `recovery`. A fresh operation is safe only when cleanup is completed and
  the provider exit was observed before readiness. Trust/sign-in/setup prompts
  require configuration first; incomplete cleanup remains unsafe. Replaying or
  reconciling this operation returns that metadata without another create or
  board scan, including after daemon restart. The initial error may include a
  bounded redacted stderr tail, but durable receipts and replay results never
  retain diagnostic text.
- `not_created` records a definite pre-create refusal; `safe_to_retry: true`
  permits a new operation after the reported cause is repaired.
- `ambiguous` means the daemon cannot confirm the upstream outcome. A single
  board read, bounded to five seconds and one MiB, reports up to 64 existing
  candidate IDs on the target machine for the parent identity triple
  (`machine`, `session_id`, `session_created_at`), which is the public board
  projection. Audit-only parent incarnations are absent there. Root starts
  report root candidates instead. Candidates are inspection aids, not
  evidence that a particular child belongs to this operation. An unavailable
  or empty board never establishes absence or permits another create;
  `safe_to_retry` remains false. The `candidate_scan` field distinguishes an
  observed board from an unavailable read.

Receipts survive daemon restart and retain only request fingerprints and
bounded session metadata, never prompts, launch environment values, or
transcripts. They are reserved under a short coordination lock before the
network call; no coordination lock spans network I/O.

Unknown UUIDv7 IDs must
be at most seven days old and no more than one minute ahead of the daemon's
clock. At or above 1024 receipts the store prunes the oldest receipts in
batches of up to 256, including recent ambiguous outcomes when necessary.
Admission repeats bounded maintenance batches until a slot is available.

A durable timestamp watermark is
advanced before deletion: unknown IDs at or below it return
`console-start-operation-expired` and can never create another child. Existing
retained receipts can still be reconciled. Thus retention can shorten the
reconciliation window under sustained volume without permitting a duplicate.

Late completion writes take the same short store lock as pruning. If pruning
already published its watermark and removed a reservation, completion reports
the created child without recreating the receipt or consuming a reclaimed slot.

Capacity pressure can expire ambiguous reconciliation evidence; its persisted
watermark still prevents any evicted operation from being submitted again.
Definite typed pre-create refusals are terminal
`not_created` outcomes. Unknown receipts or another caller incarnation return
`console-start-operation-not-found` on an explicit reconciliation only while
the ID remains within the admission window and above the expiry watermark.
Missing aged or evicted IDs return `console-start-operation-expired` with the
same inspect-Console guidance as replay; neither check calls upstream. Reconciliation uses
HTTP 201 and the existing Console start schema with the additive
`reconciliation` object; ambiguous results omit `session`.
