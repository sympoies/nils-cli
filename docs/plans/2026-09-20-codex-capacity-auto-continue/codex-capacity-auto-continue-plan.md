# Plan: Resume managed Codex sessions after capacity interruption

## Overview

Extend `agent-session`'s existing structured Codex failure and durable
auto-resume pipeline so an opted-in managed session can submit one bounded
continuation after `serverOverloaded`. Keep the complete decision and mutation
inside `sympoies/nils-cli`; Agent Console remains a contract consumer and no
OpenAI Codex change is required.

## Read First

- Primary source: docs/plans/2026-09-20-codex-capacity-auto-continue/codex-capacity-auto-continue-discussion-source.md
- Source type: discussion-to-implementation-doc
- Open questions carried into execution: none

## Scope

- In scope: trusted capacity arming, durable recovery-cause state, bounded
  capacity backoff, fixed continuation prompt, app-server submission fences,
  cancellation and restart behavior, Main Agent supervision semantics,
  contract documentation, devlog, focused tests, review, and PR delivery.
- Out of scope: terminal text parsing, UI-side prompt injection, raw Codex TUI
  support, automatic model/account switching for capacity, upstream Codex
  changes, deployment, and merging Agent Console PR #493.

## Assumptions

1. Capacity recovery is part of the existing opt-in auto-resume control; it is
   not enabled for a session whose auto-resume setting is off.
2. The installed and audited managed app-server contract remains the sole
   authoritative source of `serverOverloaded` evidence.
3. Five daemon-owned retries with the existing 30/60/120/300/600-second delays
   are sufficient to recover ordinary transient capacity without creating an
   unbounded prompt loop.

## Sprint 1: Implement structured capacity recovery

**PR grouping intent**: `per-sprint`
**Execution Profile**: `serial`

**Goal**: Deliver one reviewed nils-cli PR that safely continues an opted-in
managed Codex session after a structured capacity interruption.

**Demo/Validation**:
- Command(s): focused `nils-agent-session` Rust tests; `bash
  scripts/ci/nils-cli-checks-entrypoint.sh --local-fast`
- Verify: structured capacity evidence schedules and submits the fixed prompt
  once, every race/cancellation fence wins, repeated capacity failures remain
  bounded, and quota recovery does not regress.

### Task 1.1: Capture failing capacity-recovery tests

- **Location**:
  - `crates/agent-session/src/auto_resume.rs`
  - `crates/agent-session/src/codex_app_server.rs`
  - `crates/agent-session/tests/integration/`
- **Description**: Add regression coverage that proves the current code records
  `provider_capacity` but does not arm or submit auto-resume, then specify the
  exact positive and negative recovery matrix.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - The pre-change test fails for the missing capacity continuation behavior.
  - Negative cases cover raw prose, wrong identity, disabled auto-resume,
    pending attention, manual input, replacement runtime, and duplicate events.
- **Validation**:
  - `cargo test -p nils-agent-session auto_resume`

### Task 1.2: Generalize durable auto-resume for capacity

- **Location**:
  - `crates/agent-session/src/activity.rs`
  - `crates/agent-session/src/auto_resume.rs`
- **Description**: Add an additive recovery cause to durable auto-resume,
  atomically arm it from exact `provider_capacity` activity, preserve a bounded
  attempt chain across repeated capacity turns, and keep quota/account recovery
  behavior unchanged.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Only authoritative, correlated app-server capacity failures arm recovery.
  - The chain uses 30/60/120/300/600-second delays and terminates after five
    submitted attempts.
  - Existing state files migrate additively and old readers retain the v1
    public projection.
- **Validation**:
  - `cargo test -p nils-agent-session auto_resume`

### Task 1.3: Submit the fixed continuation through the control channel

- **Location**:
  - `crates/agent-session/src/codex_app_server.rs`
  - `crates/agent-session/src/serve.rs`
  - `crates/agent-session/src/auto_resume.rs`
- **Description**: Route due capacity recovery through the existing bound
  app-server submitter using the fixed English prompt, with the same durable
  pre-submit claim, runtime/activity/attention/account/manual-input fences, and
  unknown-outcome terminal handling as quota continuation.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 8
- **Acceptance criteria**:
  - No terminal pane write or synthetic Enter occurs.
  - Acknowledged `turn/start` completes the one attempt; an unknown outcome is
    never replayed.
  - Human or control-plane input cancellation wins before provider submission.
- **Validation**:
  - `cargo test -p nils-agent-session codex_app_server`

### Task 1.4: Align supervision and public documentation

- **Location**:
  - `crates/agent-session/src/main_agent.rs`
  - `crates/agent-session/docs/turn-state-contract.md`
  - `crates/agent-session/docs/provider-turn-signal-evidence.md`
  - `crates/agent-session/docs/specs/serve-api-v1.md`
  - `crates/agent-session/docs/specs/main-agent-orchestration-v1.md`
  - `crates/agent-session/docs/runbooks/main-agent-orchestration.md`
  - `docs/devlog/`
- **Description**: Document daemon-owned capacity recovery, ensure Main Agent
  supervision waits without issuing a competing retry, and record the shipped
  behavior in the devlog.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Supervision distinguishes scheduled daemon recovery from a capacity event
    that still requires operator attention.
  - Public projections remain metadata-only and backwards compatible.
  - Documentation states that raw terminal text never authorizes recovery.
- **Validation**:
  - `cargo test -p nils-agent-session main_agent`
  - `bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast`

## Testing Strategy

- Unit-test failure reduction, capacity arming, durable migration, retry
  sequencing, duplicate suppression, and terminal exhaustion.
- Integration-test control-channel submission, acknowledged turn IDs,
  cancellation races, daemon restart, replacement runtime, and protocol-health
  failure.
- Retain the existing quota/reset/account failover matrix as non-regression
  coverage.
- Run the repository local-fast finish-line gate once after focused tests pass.

## Risks & gotchas

- A fresh failed continuation has a new provider turn ID; the bounded capacity
  attempt counter must follow the recovery chain rather than reset per turn.
- A durable claim written before a network error cannot be safely retried; keep
  unknown submission outcomes terminal even if that reduces liveness.
- Main Agent's current `provider_capacity_attention_required` wording forbids
  automatic retry; update it narrowly so only the daemon-owned, fenced path is
  allowed and external supervisors still never resend.
- Codex versions can evolve additively; branch only on the structured enum and
  terminal correlation, never on its rendered message.

## Rollback plan

Revert the nils-cli feature PR. Existing structured capacity observation and
operator-attention behavior remain intact, while quota auto-resume continues to
use its prior code path.
