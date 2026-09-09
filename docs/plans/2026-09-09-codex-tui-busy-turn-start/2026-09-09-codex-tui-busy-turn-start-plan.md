# Plan: Fix Codex TUI Busy Turn Start

## Overview

Restore ordinary Codex TUI prompt submission for Agent Console sessions that
use a managed account. Preserve the account-mutation and exact-incarnation
fences, add privacy-safe rejection evidence, and deliver the fix through a
reviewed release and pane-preserving runtime restart.

## Read First

- Primary source: `docs/plans/2026-09-09-codex-tui-busy-turn-start/2026-09-09-codex-tui-busy-turn-start-discussion-source.md`
- Source type: discussion-to-implementation-doc
- Open questions carried into execution: none

## Scope

- In scope: live-shape regression coverage for initial and subsequent TUI
  prompts, the narrow authorization fix, bounded rejection reason telemetry,
  operator guidance, historical plan closeout, review, PR, release, fixed-fleet
  deployment, and live Agent Console acceptance.
- Out of scope: weakening account switching fences, automatic prompt replay,
  changing Codex credentials, changing provider wire shapes, or destroying and
  recreating existing operator sessions.

## Assumptions

1. The exact rejection branch can be exposed as a bounded enum without prompt,
   account, path, thread, or session identifiers.
2. A controlled freebox canary is representative because both fresh-session
   shapes reproduce before provider acceptance.
3. The authorized request covers provider issue/PR closeout, stable release,
   fixed-fleet deployment, and a pane-preserving Agent Console restart.

## Sprint 1: Diagnose and Repair Authorization

**Goal**: Make valid managed-account TUI submissions pass while invalid or
contended mutations remain fail-closed.
**Demo/Validation**:

- Commands: focused `nils-agent-session` tests and an isolated daemon canary.
- Verify: no-init first input and post-init next input start provider turns;
  negative account and incarnation cases remain rejected.

### Task 1.1: Capture live-shape regressions

- **Location**:
  - `crates/agent-session/src/codex_app_server.rs`
- **Description**: Add focused tests for the managed-account authorization
  state observed in fresh no-init and completed-init sessions.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - The new tests fail against the current implementation at the reproduced
    authorization boundary.
  - Existing contention and replacement-runtime tests remain unchanged.
- **Validation**:
  - `cargo test -p nils-agent-session tui_turn_start -- --nocapture`

### Task 1.2: Repair the authorization boundary

- **Location**:
  - `crates/agent-session/src/codex_app_server.rs`
  - related focused tests
- **Description**: Correct the minimum gate or record-authority mismatch proven
  by Task 1.1 without relaxing account mutation serialization.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Valid first and subsequent TUI turns are forwarded exactly once.
  - Queued account changes, replacement runtimes, malformed requests, and
    concurrent in-flight starts continue to reject.
- **Validation**:
  - focused unit and proxy tests

### Task 1.3: Add privacy-safe rejection evidence

- **Location**:
  - `crates/agent-session/src/codex_app_server.rs`
  - `crates/agent-session/docs/runbooks/serve-daemon.md`
- **Description**: Preserve the stable JSON-RPC response while recording one
  bounded reason code for local rejection and document diagnosis.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Evidence contains no prompt, account, path, thread, session, or credential
    values.
  - Operators can distinguish contention from malformed or stale authority.
- **Validation**:
  - focused tests and docs validation

## Sprint 2: Deliver and Verify

**Goal**: Merge, release, deploy, restart safely, and prove both live session
shapes through Agent Console.
**Demo/Validation**:

- Commands: repository local-fast gate, governed delivery, stable release,
  runtime smoke, and controlled canaries.
- Verify: provider checks and review pass, installed versions converge, panes
  survive restart, and both canaries accept a turn.

### Task 2.1: Validate, review, and merge

- **Location**:
  - repository-wide change set
- **Description**: Run the finish gate, obtain independent testing and
  maintainability review, repair findings, and merge the issue-linked PR.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Test-first evidence verifies.
  - `bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast` passes.
  - Review approves the delivered head and provider checks pass.
- **Validation**:
  - repository local-fast gate and provider read-back

### Task 2.2: Release, deploy, and live verify

- **Location**:
  - private fixed-fleet release and Agent Console runtime owner
- **Description**: Publish the next safe stable release, deploy it, preserve
  live panes through the governed restart, and execute both live canaries.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Fixed-fleet versions converge.
  - Agent Console remains active/running with `KillMode=process`.
  - Existing pane/session counts are preserved.
  - Fresh no-init and completed-init sessions accept ordinary TUI turns.
- **Validation**:
  - release receipts, runtime status/smoke, and privacy-safe live evidence

### Task 2.3: Close historical and current tracking

- **Location**:
  - `docs/plans/2026-09-08-agent-session-runtime-recovery/`
  - this plan bundle and provider records
- **Description**: Repair the stale historical execution state without
  rewriting its delivered scope, archive completed plans, and close the current
  issue with merge, release, deploy, and acceptance evidence.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Historical #1631 records its actual completed #1632/v1.28.8 delivery.
  - The current tracker closes only after live acceptance.
- **Validation**:
  - plan bundle strict validation and provider close-readiness/read-back

## Testing Strategy

- Unit: authorization branch, record identity, malformed request, contention.
- Integration: proxy request/response retention and isolated daemon canaries.
- E2E/manual: freebox fresh-session and post-init turns after fixed-fleet deploy.

## Risks & gotchas

- A permissive fallback could cross an account change; retain the mutation gate
  and exact runtime identity checks.
- A local rejection does not prove non-delivery; tests and live probes must
  assert provider-visible turn creation before any retry.
- Rejection evidence must remain a closed vocabulary with no user-controlled
  values.

## Rollback plan

- Revert the merged runtime change, publish the next governed stable patch, and
  redeploy through the fixed-fleet workflow.
- Restart only the Agent Console serve daemon with `KillMode=process`; retain
  all existing tmux panes and session records.
