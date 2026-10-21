# agent-session Documentation

Use the crate [README](../README.md) for the non-normative product overview,
common commands, and document routing. Use the documents below when operating
or integrating a specific subsystem.

## Operator runbooks

- [Session lifecycle operations](runbooks/session-lifecycle.md): sending input
  and prompt submission outcomes, clear and rebind, resume, restart, verified
  deletion, startup logs, DSH launch, account switching, model settings, and
  secret safety.
- [Lifecycle incident evidence](runbooks/lifecycle-journal.md): bounded per-session
  lifecycle records, refusal proof steps, retention, and runtime exit observations.
- [Work coordination](runbooks/work-coordination.md): coordination modes,
  declared paths, authority boundaries, advisory flow, and enforce flow.
- [Serve daemon operations](runbooks/serve-daemon.md): safe startup,
  authentication boundaries, HTTP session creation, restart survival, and the
  versioned `serve --config` file.

## Stable contracts

- [Send v1](specs/send-v1.md): submission-time continuation guards, observation
  limits, durable idempotency claims, and refusal codes.
- [Session context usage v1](specs/session-context-usage-v1.md): optional raw
  context-window facts, provider binding, bounded reads, and freshness.
- [Session display metadata v1](specs/session-display-metadata-v1.md): explicit
  roles, persisted auto/pinned titles, and exact-session revision-fenced updates.
- [Serve API v1](specs/serve-api-v1.md): HTTP and WebSocket endpoints,
  response/authentication rules, launch profiles, and session survival. Its
  endpoint map routes history, observation, automation, prompts, accounts,
  creation, terminal transport, and usage to topic contracts.
- [Native busy replies](specs/native-busy-replies.md): immediate client handoff,
  provider acknowledgment limits, and the separate explicit idle-only option.
- [Session retitle v2](specs/session-retitle-v2.md): bounded title context,
  automatic scheduling, mutation fences, provider configuration, and privacy.
- [Session retitle v3](specs/session-retitle-v3.md): bounded semantic memory,
  incremental history projection, asynchronous operations, freshness, and
  long-session privacy/fencing guarantees.
- [Session coordination v1](specs/session-coordination-v1.md): normative
  core state machines, trust, authorization, and owned children. Its contract
  map routes schemas, mailbox, runtime recovery, interfaces, and federation
  to topic contracts.
- [Scoped worker delegation v1](specs/scoped-worker-delegation-v1.md): expiring
  operation and recipient scopes for supervised workers, revocation and audits.
- [Session board v1](specs/session-board-v1.md): opt-in cross-host board
  record, closed-session ledger and cursor, relay route, aggregator query
  contract, and the `agent-session board` CLI.
- [Session public metadata v1](specs/session-public-metadata-v1.md): bounded
  revision-fenced attachment requests, replay receipts, and read-back privacy.
- [Session lineage and work v1](specs/session-lineage-work-v1.md): the
  `lineage` (parent, root, depth, starter) and `work` (program and issue
  references) members of every session record, how each start path fills
  them, and the additive `origin` (launch parent) and `workspace` (grouping
  label) members.
- [Turn-state contract](turn-state-contract.md): runtime-bound activity state,
  privacy projection, replay, and provider setup behavior.
- [Activity stream v1](specs/activity-stream-v1.md): SSE stream, replay,
  reset, flow control, and privacy contract.
- [Control-plane observation v1](specs/control-plane-observation-v1.md): the
  centralized hook/session event plane, its bounded spool and privacy budget,
  the `agent-session diagnose` bundle, and broker release publication.
- [Session maintenance v1](specs/session-maintenance-v1.md): repair and
  maintenance operation contract.
- [Session maintenance v2](specs/session-maintenance-v2.md): successor contract
  adding record-only removal for a runtime with no safe signal boundary.
- [Mail audit v1](specs/mail-audit-v1.md): owner-only, paginated mailbox
  metadata, anomalies, source journal and notification evidence.

## Evidence and migration reports

- [Provider turn-signal evidence](provider-turn-signal-evidence.md): provider
  versions and lifecycle-signal evidence behind the turn-state integration.
- [Completion migration contract](reports/agent-session-completion-migration-contract.md):
  clap-first completion coverage and verification record.
