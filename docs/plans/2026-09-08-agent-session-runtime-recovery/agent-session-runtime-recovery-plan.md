# Plan: Recover Agent Session Runtime Failures

## Overview

Prevent Agent Console's `agent-session serve` daemon from crash-looping when
historical Codex account bindings exist but tmux has no live server. Preserve
fail-closed behavior for genuine inspection failures, and document the existing
incarnation-fenced structured prompt route as the deterministic recovery for a
resumed Codex session whose raw terminal submission is rejected as locally
busy. Deliver the fix through review, release it to the fixed fleet, and verify
the live service without destroying existing tmux panes.

## Read First

- Primary source: `docs/plans/2026-09-08-agent-session-runtime-recovery/agent-session-runtime-recovery-discussion-source.md`
- Source type: discussion-to-implementation-doc
- Open questions carried into execution: none

## Scope

- In scope: meaningful regression tests for empty tmux state, a narrow snapshot
  classification fix, operator recovery documentation for local Codex busy
  submission, required validation, independent review, PR merge, stable release,
  fixed-fleet deployment, governed Agent Console restart, and live smoke checks.
- Out of scope: changing the terminal byte-stream contract, retrying a prompt
  whose provider-delivery outcome is unknown, rebuilding Agent Console's UI,
  altering Codex account binding semantics, or weakening reconnect fencing for
  ambiguous tmux failures.

## Assumptions

1. Tmux's bounded diagnostic for an absent server can be classified separately
   from generic command failure without starting or mutating a tmux server.
2. After observation establishes non-delivery and no in-progress provider turn,
   `prompt/v2` is the correct existing recovery primitive because it requires
   the exact session incarnation and returns provider acknowledgement.
3. Automatic fallback from raw terminal input is unsafe: the caller cannot
   always prove whether input reached the provider, so recovery stays explicit.
4. The currently authorized request covers issue, PR, merge, release, deploy,
   governed restart, and live verification.

## Sprint 1: Runtime Semantics and Recovery Contract

**Goal**: Distinguish an empty tmux universe from inspection failure and retain
an exact, repeatable operator recovery path for locally busy Codex input.
**Demo/Validation**:

- Commands: focused `nils-agent-session` tests.
- Verify: no-server returns an empty snapshot, genuine failure remains
  unavailable, and recovery documentation uses the exact incarnation fence.

### Task 1.1: Capture the empty-tmux regression

- **Location**:
  - `crates/agent-session/src/lib.rs`
- **Description**: Add a focused fake-tmux test that reproduces the current
  `None` snapshot when tmux reports there is no server.
- **Dependencies**:
  - none
- **Complexity**: 2
- **Acceptance criteria**:
  - The test fails against the current implementation for the intended semantic mismatch.
  - A separate case protects generic tmux failure as unavailable.
- **Validation**:
  - `cargo test -p nils-agent-session tmux_session_snapshots -- --nocapture`

### Task 1.2: Classify an absent tmux server as empty

- **Location**:
  - `crates/agent-session/src/lib.rs`
- **Description**: Return an empty batched snapshot only for tmux's bounded,
  recognized absent-server diagnostic; preserve `None` for every other
  non-success outcome.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `fence_codex_controls_before_listen` succeeds with historical bound records and no tmux server.
  - Generic command failure still produces `codex-account-reconnect-fence-unavailable`.
  - Snapshot parsing and ordinary live-session discovery remain unchanged.
- **Validation**:
  - `cargo test -p nils-agent-session tmux_session_snapshots -- --nocapture`
  - `cargo test -p nils-agent-session reconnect_fence -- --nocapture`

### Task 1.3: Document deterministic busy-input recovery

- **Location**:
  - `crates/agent-session/docs/runbooks/serve-daemon.md`
  - `crates/agent-session/docs/specs/serve-api-v1.md`
- **Description**: Add a bounded recovery procedure that first establishes
  non-delivery and no in-progress provider turn, then uses `prompt/v2` with the
  exact current session incarnation; explain why blind raw-input retries or an
  unfenced fallback are unsafe.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - Operators can recover a resumed Codex session without reconstructing its provider command.
  - The documented request uses no private identifiers or machine-specific paths.
  - The API contract remains additive and unchanged.
- **Validation**:
  - `bash scripts/ci/nils-cli-checks-entrypoint.sh --docs-only`

## Sprint 2: Delivery and Live Acceptance

**Goal**: Review, merge, release, deploy, and prove the repair on the fixed fleet.
**Demo/Validation**:

- Commands: repository local-fast gate, governed PR delivery, private stable
  release workflow, governed runtime restart, and Agent Console smoke suite.
- Verify: required checks pass, installed versions converge, the daemon remains
  active with no tmux sessions, and existing sessions survive restart.

### Task 2.1: Validate, review, and merge

- **Location**:
  - repository-wide change set
- **Description**: Run the declared finish-line gate, deliver the issue-linked
  PR without merge, obtain testing and maintainability reviews, repair valid
  findings, and merge through the retained L2 workflow.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Test-first evidence verifies cleanly.
  - `bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast` passes.
  - Independent review approves the delivered head.
  - Provider read-back confirms the PR merged.
- **Validation**:
  - `bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast`

### Task 2.2: Release, deploy, and verify live recovery

- **Location**:
  - private infrastructure release and Agent Console runtime owners
- **Description**: Select the next safe stable version, run the required dry
  run, dispatch and monitor the fixed-fleet release, restart the Agent Console
  daemon through its pane-preserving procedure, and run live acceptance.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - The stable release completes and fixed-fleet versions converge.
  - The Agent Console daemon reports active/running with `KillMode=process`.
  - The full Agent Console smoke script passes.
  - Existing live tmux panes and session records remain intact.
  - With no tmux server, a controlled daemon restart no longer crash-loops.
- **Validation**:
  - Governed release workflow receipts
  - Agent Console service status and full smoke script
  - Privacy-safe session-count comparison before and after restart

## Testing Strategy

- Unit: fake-tmux no-server, generic failure, and existing snapshot parsing.
- Integration: reconnect-fence startup with historical records and no live tmux
  server, plus the repository local-fast gate.
- E2E/manual: fixed-fleet installation, pane-preserving daemon restart, complete
  Agent Console smoke, and an empty-tmux restart probe if it can be isolated
  without disrupting active sessions.

## Risks & gotchas

- Tmux diagnostics may differ by platform or version; classification must be
  bounded to known absent-server wording and covered on Linux and macOS CI.
- Treating every exit status 1 as empty would hide permission, socket, or binary
  failures and is explicitly forbidden.
- Raw terminal submission has an ambiguous delivery boundary. Never
  automatically resubmit through `prompt/v2` after a busy or generic transport
  failure; first establish that the continuation was not accepted and no turn
  remains in progress.
- Restart validation must preserve live panes and compare privacy-safe counts;
  do not kill or recreate operator sessions to manufacture an empty state.

## Rollback plan

- Revert the merged runtime change and publish the next governed stable patch.
- Redeploy the fixed fleet and restart only the serve daemon through the
  pane-preserving runtime procedure.
- The documented `prompt/v2` recovery remains valid because it is an existing
  API contract, not a new wire dependency.
