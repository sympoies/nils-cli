# Plan: Workspace shared-crate refactor for testability and stability

## Overview
This plan defines a full-workspace refactor pass to scan every crate, identify reusable logic that should move to `nils-common`, `nils-term`, or `nils-test-support`, and execute migrations in controlled batches.  
The primary goals are higher testability, lower duplication, and more stable runtime behavior without breaking CLI output/exit-code contracts.  
Work is sequenced as sprint-level integration gates: inventory first, then runtime helper convergence, progress contract alignment, test harness convergence, final extraction pass, and shared-crate docs refresh.

## Scope
- In scope:
  - All workspace crates under `crates/*` (25 crates total).
  - Shared-helper adoption and extraction targeting `crates/nils-common`, `crates/nils-term`, and `crates/nils-test-support`.
  - Characterization/contract test hardening required to keep behavior parity during migrations.
  - Final README/doc updates for shared crates.
- Out of scope:
  - New end-user features unrelated to helper reuse.
  - UI copy redesign unrelated to parity-safe refactors.
  - Cross-repo dependency changes outside this workspace.

## Assumptions
1. Contract fidelity remains top priority: output text, warning style, color behavior, and exit semantics must remain stable unless explicitly approved.
2. Each sprint is an integration gate; no cross-sprint execution parallelism is planned.
3. Shared crates may add new domain-neutral APIs only when duplication is observed in two or more crates.
4. Risky migrations (auth paths, process wrappers, atomic file behavior) require characterization tests before extraction.
5. Final delivery includes required checks and workspace coverage gate per `DEVELOPMENT.md`.

## Success criteria
- Every crate has an explicit inventory row and classification (`migrate`, `extend-shared`, `keep-local`, or `defer` with reason).
- Repeated runtime primitives converge into `nils-common` with crate-local adapters preserving UX/exit semantics.
- Progress behavior is consistently mediated by `nils-term` where progress is needed.
- Test harness duplication is reduced via `nils-test-support` (`EnvGuard`, `GlobalStateLock`, `cmd`, `bin`, `git`, `fs`, stubs).
- Final sprint updates shared-crate READMEs and docs so APIs and migration guidance match shipped behavior.

## Workspace crate baseline (scan target: all crates)
| Crate | Initial signals | Likely shared target |
| --- | --- | --- |
| `agent-docs` | test env/path setup patterns | `nils-test-support`, `nils-common` |
| `api-gql` | progress + API command orchestration | `nils-term`, `nils-test-support` |
| `api-grpc` | progress + grpc test stubs | `nils-term`, `nils-test-support` |
| `api-rest` | progress + request runner | `nils-term`, `nils-test-support` |
| `api-test` | orchestration and suite runner tests | `nils-term`, `nils-test-support` |
| `api-testing-core` | env mutation + grpc script setup | `nils-common`, `nils-test-support`, `nils-term` |
| `api-websocket` | progress + integration harness | `nils-term`, `nils-test-support` |
| `cli-template` | baseline wiring crate | `nils-common`, `nils-term`, `nils-test-support` |
| `codex-cli` | git/process/env/fs helper duplication hotspots | `nils-common`, `nils-test-support`, `nils-term` |
| `fzf-cli` | test stub + executable helper duplication | `nils-test-support`, `nils-term`, `nils-common` |
| `gemini-cli` | git/process/env/fs + heavy test helper duplication | `nils-common`, `nils-test-support` |
| `git-cli` | local git helper patterns + test env handling | `nils-common`, `nils-test-support` |
| `git-lock` | runtime git wrapper patterns | `nils-common`, `nils-term`, `nils-test-support` |
| `git-scope` | git rendering/probing and harness overlap | `nils-common`, `nils-term`, `nils-test-support` |
| `git-summary` | git command wrappers + progress + harness overlap | `nils-common`, `nils-term`, `nils-test-support` |
| `image-processing` | progress + command orchestration tests | `nils-term`, `nils-test-support`, `nils-common` |
| `macos-agent` | command/path handling patterns in tests | `nils-common`, `nils-test-support` |
| `memo-cli` | `NO_COLOR` behavior adapter and testability | `nils-common`, `nils-test-support` |
| `nils-common` | shared runtime primitive owner | maintain + extend |
| `nils-term` | shared progress primitive owner | maintain + extend |
| `nils-test-support` | shared test primitive owner | maintain + extend |
| `plan-issue-cli` | helper reuse + deterministic harness patterns | `nils-common`, `nils-test-support` |
| `plan-tooling` | git/process shared wrapper opportunities | `nils-common`, `nils-term`, `nils-test-support` |
| `screen-record` | stub/path/env test helper patterns | `nils-test-support`, `nils-common` |
| `semantic-commit` | git/process/env helper overlap | `nils-common`, `nils-term`, `nils-test-support` |

## Sprint 1: Workspace-wide inventory and decision matrix
**Goal**: Build a deterministic crate-by-crate manifest and dependency graph before touching shared APIs.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `parallel-x2` (parallel width 2).
**Demo/Validation**:
- Command(s):
  - `cargo run -p nils-plan-tooling --bin plan-tooling -- validate --file docs/plans/workspace-shared-crates-testability-stability-refactor-plan.md`
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv --out "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv"`
- Verify:
  - Audit artifacts list all workspace crates with shared-helper opportunities and classifications.
  - Each candidate row is mapped to an owning task/sprint.
**Sprint scorecard**:
- `TotalComplexity`: 16
- `CriticalPathComplexity`: 13
- `MaxBatchWidth`: 2
- `OverlapHotspots`: `scripts/dev/workspace-shared-crate-audit.sh` and audit outputs under `$AGENT_HOME/out/workspace-shared-audit/`.
**Parallelizable tasks**:
- `Task 1.2` and `Task 1.3` can run in parallel after `Task 1.1`.

### Task 1.1: Build full crate matrix and scan automation script
- **Location**:
  - `scripts/dev/workspace-shared-crate-audit.sh`
  - `$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv`
  - `$AGENT_HOME/out/workspace-shared-audit/crate-matrix.md`
- **Description**: Create a repeatable scanner that inventories all workspace crates, current shared-crate dependency/use status, and duplication signals (process/git/env/fs/progress/test harness patterns).
- **Dependencies**:
  - none
- **Complexity**: 5
- **Complexity notes**: New scanner + output schema must stay deterministic for future re-runs in CI.
- **Acceptance criteria**:
  - Output includes all 25 crates exactly once.
  - Output schema includes `crate`, `target_shared_crate`, `signal`, `proposed_action`, `owner_task`.
  - Script writes only under `$AGENT_HOME/out/workspace-shared-audit/`.
- **Validation**:
  - `mkdir -p "$AGENT_HOME/out/workspace-shared-audit"`
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv --out "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv"`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv"`
  - `awk -F '\t' 'NR>1 {print $1}' "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv" | sort -u | wc -l | tr -d ' ' | rg '^25$'`

### Task 1.2: Generate per-target hotspot reports (`nils-common` / `nils-term` / `nils-test-support`)
- **Location**:
  - `$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-common.md`
  - `$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-term.md`
  - `$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-test-support.md`
  - `$AGENT_HOME/out/workspace-shared-audit/hotspots-index.tsv`
- **Description**: Group scan findings by target shared crate and rank hotspots by duplication breadth and regression risk.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Complexity notes**: Requires consistent severity heuristics so later sprint planning stays stable.
- **Acceptance criteria**:
  - Each target report includes crate/file references and risk level.
  - Hotspots include at least `high|medium|low` ranking with rationale.
  - Report format is machine-readable enough to diff between runs.
- **Validation**:
  - `for f in "$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-common.md" "$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-term.md" "$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-test-support.md"; do test -s "$f"; done`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/hotspots-index.tsv"`
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv --out "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.rerun.tsv"`
  - `cmp -s "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv" "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.rerun.tsv"`
  - `rg -n 'high|medium|low|rationale' "$AGENT_HOME/out/workspace-shared-audit"/hotspots-*.md`

### Task 1.3: Define migration decision rubric and parity guardrails
- **Location**:
  - `$AGENT_HOME/out/workspace-shared-audit/decision-rubric.md`
  - `AGENTS.md`
  - `DEVELOPMENT.md`
- **Description**: Write explicit rules for classifying findings as `migrate`, `extend-shared`, `keep-local`, or `defer`, with parity and testability gates tied to repository policy.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Complexity notes**: Mostly policy synthesis; complexity comes from edge-case clarity.
- **Acceptance criteria**:
  - Rubric names contract gates (output/exit/color/JSON stability).
  - Rubric names test gates (characterization + required checks + coverage).
  - Every classification has an explicit decision rule.
- **Validation**:
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/decision-rubric.md"`
  - `rg -n 'migrate|extend-shared|keep-local|defer' "$AGENT_HOME/out/workspace-shared-audit/decision-rubric.md"`
  - `rg -n 'output|exit|color|JSON|coverage|characterization' "$AGENT_HOME/out/workspace-shared-audit/decision-rubric.md"`

### Task 1.4: Freeze task ownership graph and execution lanes from audit output
- **Location**:
  - `$AGENT_HOME/out/workspace-shared-audit/task-lanes.tsv`
  - `docs/plans/workspace-shared-crates-testability-stability-refactor-plan.md`
- **Description**: Translate findings into an executable ownership graph so each migration path has clear dependencies and minimal file-overlap conflicts.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Complexity notes**: Requires balancing sprint sequencing and overlap risk, not just listing tasks.
- **Acceptance criteria**:
  - Every hotspot row has a non-empty owning task ID.
  - Parallel lanes avoid overlapping high-churn files unless intentionally serialized.
  - Lane assignment is deterministic across repeated scan runs.
- **Validation**:
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/task-lanes.tsv"`
  - `awk -F '\t' 'NR>1 && $5=="" {print $0}' "$AGENT_HOME/out/workspace-shared-audit/task-lanes.tsv" | wc -l | tr -d ' ' | rg '^0$'`
  - `cp "$AGENT_HOME/out/workspace-shared-audit/task-lanes.tsv" "$AGENT_HOME/out/workspace-shared-audit/task-lanes.baseline.tsv"`
  - `bash scripts/dev/workspace-shared-crate-audit.sh --emit-lanes --in "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv" --out "$AGENT_HOME/out/workspace-shared-audit/task-lanes.rerun.tsv"`
  - `diff -u "$AGENT_HOME/out/workspace-shared-audit/task-lanes.baseline.tsv" "$AGENT_HOME/out/workspace-shared-audit/task-lanes.rerun.tsv"`
  - `rg -n 'Task 2\\.|Task 3\\.|Task 4\\.|Task 5\\.|Task 6\\.' "$AGENT_HOME/out/workspace-shared-audit/task-lanes.tsv"`

## Sprint 2: `nils-common` runtime primitive convergence
**Goal**: Remove repeated runtime helpers by routing process/git/env/fs primitives through `nils-common` with parity-safe crate-local adapters.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `parallel-x2` (parallel width 2).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-git-lock`
  - `cargo test -p nils-git-scope`
  - `cargo test -p nils-semantic-commit`
  - `cargo test -p nils-plan-tooling`
- Verify:
  - Runtime helper duplication in target crates is reduced without changing user-facing contracts.
  - Shared helper migrations are covered by characterization tests.
**Sprint scorecard**:
- `TotalComplexity`: 17
- `CriticalPathComplexity`: 13
- `MaxBatchWidth`: 2
- `OverlapHotspots`: `crates/codex-cli/src/auth/*`, `crates/gemini-cli/src/auth/*`, and `crates/*/src/*git*`.
**Parallelizable tasks**:
- `Task 2.1` and `Task 2.2` can run in parallel before `Task 2.3`.

### Task 2.1: Standardize process/git command plumbing through `nils-common`
- **Location**:
  - `crates/git-lock/src/git.rs`
  - `crates/git-scope/src/git_cmd.rs`
  - `crates/semantic-commit/src/commit.rs`
  - `crates/semantic-commit/src/staged_context.rs`
  - `crates/plan-tooling/src/validate.rs`
  - `crates/git-cli/src/commit_shared.rs`
  - `crates/nils-common/src/process.rs`
  - `crates/nils-common/src/git.rs`
- **Description**: Replace repeated low-level command plumbing with shared process/git wrappers while preserving crate-local error messages and exit mapping.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 4
- **Complexity notes**: Medium risk due to subtle stderr/stdout and return-code differences across crates.
- **Acceptance criteria**:
  - Identified repeated process/git helpers are migrated or explicitly marked `keep-local`.
  - Command failure formatting remains contract-compatible.
  - No new shell-invocation regressions in target crate tests.
- **Validation**:
  - `cargo test -p nils-git-lock`
  - `cargo test -p nils-git-scope`
  - `cargo test -p nils-semantic-commit`
  - `cargo test -p nils-plan-tooling`

### Task 2.2: Converge env and color handling on `nils_common::env`
- **Location**:
  - `crates/memo-cli/src/output/text.rs`
  - `crates/codex-cli/src/starship/mod.rs`
  - `crates/codex-cli/src/starship/render.rs`
  - `crates/gemini-cli/src/starship/mod.rs`
  - `crates/gemini-cli/src/starship/render.rs`
  - `crates/git-scope/src/main.rs`
  - `crates/nils-common/src/env.rs`
- **Description**: Remove direct env parsing duplication (`NO_COLOR` and related flags) by using shared env helpers with crate-local adapters where semantics intentionally differ.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 4
- **Complexity notes**: Small API changes can impact color behavior; parity tests are mandatory.
- **Acceptance criteria**:
  - Direct `NO_COLOR` parsing is centralized through shared helpers or documented local exceptions.
  - Existing edge semantics (for example empty `NO_COLOR`) remain explicit and tested.
  - No color-mode regressions in affected test suites.
- **Validation**:
  - `cargo test -p nils-memo-cli text_output`
  - `cargo test -p nils-codex-cli starship_cached`
  - `cargo test -p nils-gemini-cli starship_cached`
  - `if rg -n 'std::env::var(_os)?\\(\"NO_COLOR\"\\)' crates/memo-cli/src crates/codex-cli/src crates/gemini-cli/src crates/git-scope/src; then echo 'unexpected direct NO_COLOR reads remain'; exit 1; fi`

### Task 2.3: Converge atomic file/timestamp/hash primitives through `nils-common::fs`
- **Location**:
  - `crates/codex-cli/src/fs.rs`
  - `crates/gemini-cli/src/fs.rs`
  - `crates/gemini-cli/src/auth/mod.rs`
  - `crates/gemini-cli/src/auth/save.rs`
  - `crates/gemini-cli/src/auth/remove.rs`
  - `crates/codex-cli/src/auth/save.rs`
  - `crates/codex-cli/src/auth/remove.rs`
  - `crates/nils-common/src/fs.rs`
  - `crates/nils-common/README.md`
- **Description**: Replace duplicated file primitives with `nils-common::fs` and preserve crate-local context formatting for auth/rate-limit flows.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Complexity notes**: Highest risk in sprint due to permission/atomicity semantics.
- **Acceptance criteria**:
  - Shared fs primitives are used for migrated flows.
  - Caller crates keep current error mapping behavior unless explicitly approved.
  - Auth/rate-limit tests continue to pass with unchanged contracts.
- **Validation**:
  - `cargo test -p nils-codex-cli auth_save`
  - `cargo test -p nils-codex-cli auth_remove`
  - `cargo test -p nils-gemini-cli auth_save`
  - `cargo test -p nils-gemini-cli auth_remove`
  - `cargo test -p nils-gemini-cli auth_refresh`

### Task 2.4: Add runtime characterization coverage for helper migrations
- **Location**:
  - `crates/codex-cli/tests/auth_json_contract.rs`
  - `crates/gemini-cli/tests/auth_json_contract.rs`
  - `crates/git-scope/tests/characterization_commands.rs`
  - `crates/git-lock/tests/diff_tag.rs`
  - `crates/semantic-commit/tests/commit.rs`
- **Description**: Add and update contract tests that pin output text, warning behavior, and exit paths impacted by shared-helper migrations.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 4
- **Complexity notes**: Mostly test additions, but broad crate surface.
- **Acceptance criteria**:
  - Tests explicitly cover migrated code paths.
  - No unreviewed output-contract drift in human or JSON modes.
  - Regression snapshots are deterministic.
- **Validation**:
  - `cargo test -p nils-codex-cli auth_json_contract`
  - `cargo test -p nils-gemini-cli auth_json_contract`
  - `cargo test -p nils-git-scope characterization_commands`
  - `cargo test -p nils-semantic-commit --test commit`

## Sprint 3: `nils-term` progress contract alignment
**Goal**: Ensure progress behavior is consistently mediated by `nils-term` and remains safe for machine-readable output modes.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-term`
  - `cargo test -p nils-api-testing-core`
  - `cargo test -p nils-api-test`
  - `cargo test -p nils-fzf-cli`
- Verify:
  - All progress-bearing commands use `nils-term` consistently.
  - JSON/machine mode output remains uncontaminated by progress text.
**Sprint scorecard**:
- `TotalComplexity`: 15
- `CriticalPathComplexity`: 15
- `MaxBatchWidth`: 1
- `OverlapHotspots`: `crates/*/src/*progress*` and command entrypoints that toggle progress in API CLIs.
**Parallelizable tasks**:
- none (intentional serial sequence because behavior contracts overlap).

### Task 3.1: Complete progress behavior audit for all crates
- **Location**:
  - `$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-term.md`
  - `$AGENT_HOME/out/workspace-shared-audit/progress-policy.tsv`
  - `Cargo.toml`
  - `crates/api-testing-core/src/suite/runner/progress.rs`
  - `crates/api-test/src/main.rs`
  - `crates/fzf-cli/src/file.rs`
  - `crates/git-summary/src/summary.rs`
- **Description**: Review all crates for long-running command paths and classify whether they should adopt, keep, or explicitly avoid progress rendering.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 3
- **Complexity notes**: Audit breadth is high but logic is straightforward.
- **Acceptance criteria**:
  - Every crate receives a `progress_policy` entry in the audit.
  - Keep-local decisions include explicit rationale.
  - Candidate migration files are enumerated for sprint tasks.
  - `progress-policy.tsv` contains exactly 25 unique crate rows with non-empty policy values.
- **Validation**:
  - `rg -n 'progress_policy|adopt|keep-local|no-progress' "$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-term.md"`
  - `awk 'END {print NR}' "$AGENT_HOME/out/workspace-shared-audit/hotspots-nils-term.md" | rg '^[1-9]'`
  - `awk -F '\t' 'NR>1 {print $1}' "$AGENT_HOME/out/workspace-shared-audit/progress-policy.tsv" | sort -u | wc -l | tr -d ' ' | rg '^25$'`
  - `awk -F '\t' 'NR>1 && $2=="" {print $0}' "$AGENT_HOME/out/workspace-shared-audit/progress-policy.tsv" | wc -l | tr -d ' ' | rg '^0$'`

### Task 3.2: Extend `nils-term` API only where cross-crate gaps are proven
- **Location**:
  - `crates/nils-term/src/progress.rs`
  - `crates/nils-term/src/lib.rs`
  - `crates/nils-term/README.md`
  - `crates/nils-term/docs/README.md`
- **Description**: Add minimal API extensions (if needed) to cover repeated progress patterns discovered in audit, while preserving stderr/TTY safety defaults.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Complexity notes**: Public API work requires careful compatibility review.
- **Acceptance criteria**:
  - New API surface is domain-neutral and documented.
  - Existing callers compile without behavior regressions.
  - Unit tests cover new options and fallback behavior.
- **Validation**:
  - `cargo test -p nils-term`
  - `cargo test -p nils-term progress`
  - `rg -n 'Progress|ProgressOptions|ProgressEnabled' crates/nils-term/README.md`

### Task 3.3: Migrate progress-bearing command paths to unified `nils-term` usage
- **Location**:
  - `crates/api-gql/src/commands/call.rs`
  - `crates/api-grpc/src/commands/call.rs`
  - `crates/api-rest/src/commands/call.rs`
  - `crates/api-websocket/src/commands/call.rs`
  - `crates/api-test/src/main.rs`
  - `crates/fzf-cli/src/file.rs`
  - `crates/image-processing/src/processing.rs`
  - `crates/git-lock/src/list.rs`
  - `crates/git-summary/src/summary.rs`
  - `crates/codex-cli/src/rate_limits/mod.rs`
  - `crates/plan-tooling/src/validate.rs`
  - `crates/semantic-commit/src/commit.rs`
- **Description**: Apply the audited progress policy by converging progress instantiation/options on shared `nils-term` helpers in participating crates.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 4
- **Complexity notes**: Wide file touch with low-to-medium per-file logic.
- **Acceptance criteria**:
  - Progress initialization patterns are consistent across participating crates.
  - Non-interactive and JSON paths keep clean stdout behavior.
  - Tests for progress-on/off modes remain deterministic.
- **Validation**:
  - `cargo test -p nils-api-testing-core`
  - `cargo test -p nils-api-test`
  - `cargo test -p nils-fzf-cli`
  - `cargo test -p nils-git-summary`

### Task 3.4: Add progress contract tests (TTY/off/json/no-color)
- **Location**:
  - `crates/api-testing-core/tests/suite_runner_loopback.rs`
  - `crates/api-testing-core/tests/suite_runner_websocket_matrix.rs`
  - `crates/api-test/tests/e2e.rs`
  - `crates/git-scope/tests/progress_opt_in.rs`
  - `crates/fzf-cli/tests/edge_cases.rs`
  - `crates/image-processing/tests/core_flows.rs`
- **Description**: Add/refresh tests asserting progress suppression in machine outputs and stable behavior for TTY and non-TTY execution contexts.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 3
- **Complexity notes**: Contract assertions are simple; ensuring deterministic capture is the main challenge.
- **Acceptance criteria**:
  - JSON output tests verify no progress leakage to stdout.
  - TTY and non-TTY behavior differences are explicitly tested.
  - `NO_COLOR` behavior remains compatible with shared env policy.
- **Validation**:
  - `cargo test -p nils-api-test`
  - `cargo test -p nils-api-testing-core suite_runner`
  - `cargo test -p nils-git-scope progress_opt_in`

## Sprint 4: `nils-test-support` convergence for deterministic tests
**Goal**: Replace bespoke test harness code with shared test primitives to reduce flakiness and simplify maintenance.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `parallel-x2` (parallel width 2).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-codex-cli`
  - `cargo test -p nils-gemini-cli`
  - `cargo test -p nils-agent-docs`
  - `cargo test -p nils-api-testing-core`
  - `cargo test -p nils-screen-record`
- Verify:
  - Env/path/git/script/bin test helpers converge on `nils-test-support`.
  - Flaky global-state interactions are reduced through shared locking/guards.
**Sprint scorecard**:
- `TotalComplexity`: 18
- `CriticalPathComplexity`: 13
- `MaxBatchWidth`: 2
- `OverlapHotspots`: `crates/gemini-cli/tests/*`, `crates/codex-cli/tests/*`, and shared fixture helpers in `tests/common.rs` files.
**Parallelizable tasks**:
- `Task 4.1` and `Task 4.2` can run in parallel before `Task 4.3`.

### Task 4.1: Replace raw env mutation and local guards with shared guard primitives
- **Location**:
  - `crates/gemini-cli/src/auth/refresh.rs`
  - `crates/gemini-cli/src/auth/save.rs`
  - `crates/codex-cli/src/auth/refresh.rs`
  - `crates/codex-cli/src/auth/save.rs`
  - `crates/semantic-commit/src/commit.rs`
  - `crates/api-testing-core/src/grpc/runner.rs`
  - `crates/git-cli/src/commit.rs`
  - `crates/nils-test-support/src/lib.rs`
- **Description**: Replace local env guard code and raw env mutation patterns with `GlobalStateLock`, `EnvGuard`, and related shared helpers where test-only behavior is intended.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 5
- **Complexity notes**: High breadth across source-level test modules and integration suites.
- **Acceptance criteria**:
  - Raw env mutation use is reduced to documented exceptional cases only.
  - Shared guard usage is consistent across touched test paths.
  - Parallel test interference risk is reduced with lock discipline.
- **Validation**:
  - `cargo test -p nils-gemini-cli auth_refresh`
  - `cargo test -p nils-codex-cli auth_refresh`
  - `cargo test -p nils-semantic-commit --test staged_context`
  - `cargo test -p nils-api-testing-core suite_runner_grpc_matrix`

### Task 4.2: Replace manual executable/PATH/bin helpers with shared stubs/fs/cmd/bin utilities
- **Location**:
  - `crates/fzf-cli/tests/git_commands.rs`
  - `crates/gemini-cli/tests/agent_prompt.rs`
  - `crates/codex-cli/tests/agent_prompt.rs`
  - `crates/screen-record/tests/linux_unit.rs`
  - `crates/git-summary/tests/cli_paths.rs`
  - `crates/nils-test-support/src/fs.rs`
  - `crates/nils-test-support/src/stubs.rs`
  - `crates/nils-test-support/src/bin.rs`
  - `crates/nils-test-support/src/cmd.rs`
- **Description**: Replace repeated executable-writer, PATH-prepend, and binary-resolution patterns with shared helper primitives.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 5
- **Complexity notes**: Medium risk due to platform-specific executable bit behavior.
- **Acceptance criteria**:
  - Common helper calls replace duplicated local patterns in targeted tests.
  - Platform guards remain correct on Unix/non-Unix where applicable.
  - Test helper readability is maintained or improved.
- **Validation**:
  - `cargo test -p nils-fzf-cli`
  - `cargo test -p nils-screen-record linux_unit`
  - `cargo test -p nils-codex-cli --test agent_prompt`
  - `cargo test -p nils-gemini-cli --test agent_prompt`

### Task 4.3: Migrate manual git test setup to `nils_test_support::git`
- **Location**:
  - `crates/agent-docs/tests/env_paths.rs`
  - `crates/codex-cli/tests/agent_commit.rs`
  - `crates/gemini-cli/tests/agent_commit_fallback.rs`
  - `crates/git-lock/tests/diff_tag.rs`
  - `crates/git-lock/tests/common.rs`
  - `crates/git-scope/tests/help_outside_repo.rs`
  - `crates/git-scope/tests/edge_cases.rs`
  - `crates/nils-test-support/src/git.rs`
- **Description**: Consolidate repeated temp-repo initialization and git command setup patterns into shared git test helpers, preserving existing assertions.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 4
- **Complexity notes**: Moderate because existing tests may depend on subtle repo state setup order.
- **Acceptance criteria**:
  - Shared git helper usage replaces repeated inline setup in targeted tests.
  - Existing fallback and edge-case assertions continue to pass.
  - No hidden dependency on local machine git config remains.
- **Validation**:
  - `cargo test -p nils-agent-docs env_paths`
  - `cargo test -p nils-codex-cli --test agent_commit`
  - `cargo test -p nils-gemini-cli --test agent_commit_fallback`
  - `cargo test -p nils-git-lock`
  - `cargo test -p nils-git-scope`

### Task 4.4: Stabilize and de-flake test concurrency with shared lock conventions
- **Location**:
  - `crates/plan-issue-cli/tests/common.rs`
  - `crates/git-lock/tests/common.rs`
  - `crates/git-scope/tests/common.rs`
  - `crates/semantic-commit/tests/common.rs`
  - `crates/screen-record/tests/common.rs`
  - `crates/nils-test-support/README.md`
  - `$AGENT_HOME/out/workspace-shared-audit/flaky-risk-report.md`
- **Description**: Define and apply lock/guard usage conventions for global mutable state, then track before/after flaky risk findings.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 4
- **Complexity notes**: Convention rollout across many test modules; minimal runtime risk.
- **Acceptance criteria**:
  - Shared lock usage conventions are documented with examples.
  - High-risk test modules are annotated and migrated.
  - Flaky-risk report includes pre/post status.
- **Validation**:
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/flaky-risk-report.md"`
  - `rg -n 'GlobalStateLock|EnvGuard|CwdGuard|StubBinDir' crates/*/tests/common.rs crates/nils-test-support/README.md`

## Sprint 5: Cross-crate duplicate extraction and closeout verification
**Goal**: Extract unresolved repeated patterns into shared crates and close the audit loop with explicit migration status for every finding.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Remaining top duplication patterns are either extracted or explicitly documented with rationale.
  - Required checks and coverage gate pass for refactor branch.
**Sprint scorecard**:
- `TotalComplexity`: 13
- `CriticalPathComplexity`: 13
- `MaxBatchWidth`: 1
- `OverlapHotspots`: shared crates (`nils-common`, `nils-term`, `nils-test-support`) and multi-crate migration call sites.
**Parallelizable tasks**:
- none (intentional serial sequencing for cross-crate contract safety).

### Task 5.1: Rank unresolved duplicate patterns and approve extraction set
- **Location**:
  - `$AGENT_HOME/out/workspace-shared-audit/unresolved-patterns.md`
  - `$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv`
- **Description**: Identify unresolved duplication patterns affecting at least three crates and finalize which ones should be extracted in this cycle.
- **Dependencies**:
  - Task 4.4
- **Complexity**: 3
- **Complexity notes**: Decision-heavy task; low coding complexity.
- **Acceptance criteria**:
  - Unresolved patterns include frequency, risk, and target shared crate.
  - Each pattern is marked `extract-now` or `defer` with reason.
  - Output feeds directly into Task 5.2 implementation list.
- **Validation**:
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/unresolved-patterns.md"`
  - `rg -n 'extract-now|defer|frequency|target' "$AGENT_HOME/out/workspace-shared-audit/unresolved-patterns.md"`

### Task 5.2: Implement shared APIs for approved unresolved patterns
- **Location**:
  - `crates/nils-common/src/lib.rs`
  - `crates/nils-common/src/process.rs`
  - `crates/nils-common/src/fs.rs`
  - `crates/nils-term/src/lib.rs`
  - `crates/nils-term/src/progress.rs`
  - `crates/nils-test-support/src/lib.rs`
  - `crates/nils-test-support/src/cmd.rs`
  - `crates/nils-test-support/src/git.rs`
  - `crates/nils-common/README.md`
  - `crates/nils-term/README.md`
  - `crates/nils-test-support/README.md`
- **Description**: Add narrowly scoped shared APIs for approved patterns and back them with deterministic unit tests before downstream migrations.
- **Dependencies**:
  - Task 5.1
- **Complexity**: 4
- **Complexity notes**: API design must remain domain-neutral and backward-compatible.
- **Acceptance criteria**:
  - New APIs are documented and tested in owning shared crates.
  - No crate-specific UX policy leaks into shared crates.
  - API naming and error surfaces are stable and reviewable.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo test -p nils-term`
  - `cargo test -p nils-test-support`

### Task 5.3: Migrate first-wave consumers and close manifest classifications
- **Location**:
  - `crates/codex-cli/src/agent/commit.rs`
  - `crates/gemini-cli/src/agent/commit.rs`
  - `crates/git-cli/src/commit_shared.rs`
  - `crates/plan-issue-cli/src/task_spec.rs`
  - `crates/fzf-cli/tests/git_commands.rs`
  - `crates/api-testing-core/tests/suite_runner_grpc_matrix.rs`
  - `$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv`
  - `$AGENT_HOME/out/workspace-shared-audit/closeout.md`
- **Description**: Migrate selected consumers to newly added shared APIs and update the audit matrix so every finding has a final classification and rationale.
- **Dependencies**:
  - Task 5.2
- **Complexity**: 3
- **Complexity notes**: Broad migration touch; keep each change behavior-preserving.
- **Acceptance criteria**:
  - First-wave consumer migrations pass targeted crate tests.
  - No audit rows remain unclassified.
  - Closeout report summarizes migration impact and deferred items.
- **Validation**:
  - `awk -F '\t' 'NR>1 && $4=="" {print $0}' "$AGENT_HOME/out/workspace-shared-audit/crate-matrix.tsv" | wc -l | tr -d ' ' | rg '^0$'`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/closeout.md"`
  - `rg -n 'migrated|keep-local|defer' "$AGENT_HOME/out/workspace-shared-audit/closeout.md"`

### Task 5.4: Run full required checks + coverage gate for refactor branch
- **Location**:
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `target/coverage/lcov.info`
  - `$AGENT_HOME/out/workspace-shared-audit/final-checks.md`
- **Description**: Execute mandatory checks and coverage gate, then store concise pass/fail summaries tied to this plan.
- **Dependencies**:
  - Task 5.3
- **Complexity**: 3
- **Complexity notes**: Operational gate task; complexity comes from multi-crate validation time.
- **Acceptance criteria**:
  - Required checks script passes.
  - Coverage gate remains `>= 85%`.
  - Final checks summary documents command outcomes and any retries.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/final-checks.md"`

## Sprint 6: Shared-crate README and docs refresh (final sprint)
**Goal**: Update and verify documentation for `nils-common`, `nils-term`, and `nils-test-support` after refactor convergence.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
- Verify:
  - Shared-crate READMEs and docs accurately reflect APIs, migration guidance, and examples.
  - Docs placement policy is fully respected.
**Sprint scorecard**:
- `TotalComplexity`: 10
- `CriticalPathComplexity`: 10
- `MaxBatchWidth`: 1
- `OverlapHotspots`: `crates/nils-common/README.md`, `crates/nils-term/README.md`, `crates/nils-test-support/README.md`, and crate-local docs indexes.
**Parallelizable tasks**:
- none (intentional serial sequence for doc consistency).

### Task 6.1: Audit shared-crate docs for accuracy gaps after migrations
- **Location**:
  - `crates/nils-common/README.md`
  - `crates/nils-term/README.md`
  - `crates/nils-test-support/README.md`
  - `crates/nils-common/docs/README.md`
  - `crates/nils-term/docs/README.md`
  - `crates/nils-test-support/docs/README.md`
  - `$AGENT_HOME/out/workspace-shared-audit/shared-docs-gap-report.md`
- **Description**: Compare implemented APIs and migration outcomes against current shared-crate docs; record missing or outdated sections.
- **Dependencies**:
  - Task 5.4
- **Complexity**: 3
- **Complexity notes**: Review-heavy task with broad surface.
- **Acceptance criteria**:
  - Gap list covers all three shared crates.
  - Each gap has a concrete doc update action.
  - No undocumented public API additions remain.
- **Validation**:
  - `rg -n '^pub (fn|struct|enum|type)' crates/nils-common/src crates/nils-term/src crates/nils-test-support/src > "$AGENT_HOME/out/workspace-shared-audit/public-api.txt"`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/public-api.txt"`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/shared-docs-gap-report.md"`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/closeout.md"`

### Task 6.2: Update READMEs, docs indexes, and migration guidance for shared crates
- **Location**:
  - `crates/nils-common/README.md`
  - `crates/nils-common/docs/README.md`
  - `crates/nils-term/README.md`
  - `crates/nils-term/docs/README.md`
  - `crates/nils-test-support/README.md`
  - `crates/nils-test-support/docs/README.md`
- **Description**: Apply doc updates covering API purpose, usage examples, migration conventions, and keep-local boundaries for each shared crate.
- **Dependencies**:
  - Task 6.1
- **Complexity**: 4
- **Complexity notes**: Medium due to cross-doc consistency and example accuracy requirements.
- **Acceptance criteria**:
  - READMEs include up-to-date module inventories and sample usage.
  - Migration guidance reflects current decision rubric and parity constraints.
  - Docs indexes link to new/changed pages with no stale references.
- **Validation**:
  - `rg -n 'Migration|What belongs|What stays crate-local|Docs index' crates/nils-common/README.md crates/nils-term/README.md crates/nils-test-support/README.md`
  - `rg -n '\\[.*\\]\\(.*\\)' crates/nils-common/docs/README.md crates/nils-term/docs/README.md crates/nils-test-support/docs/README.md`

### Task 6.3: Validate docs policy + docs-only checks and publish final docs checklist
- **Location**:
  - `docs/specs/crate-docs-placement-policy.md`
  - `scripts/ci/docs-placement-audit.sh`
  - `$AGENT_HOME/out/workspace-shared-audit/shared-docs-final-checklist.md`
- **Description**: Run docs placement and docs-only required checks, then record a final checklist for reviewers.
- **Dependencies**:
  - Task 6.2
- **Complexity**: 3
- **Complexity notes**: Low implementation complexity; strict compliance gating.
- **Acceptance criteria**:
  - Docs placement audit passes strict mode.
  - Docs-only required checks pass.
  - Final checklist summarizes updated files and verification commands.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
  - `test -s "$AGENT_HOME/out/workspace-shared-audit/shared-docs-final-checklist.md"`

## Testing Strategy
- Unit:
  - Shared crate API tests in `nils-common`, `nils-term`, `nils-test-support`.
  - Characterization tests for migrated helper behavior in consuming crates.
- Integration:
  - Per-crate integration suites for auth/process/progress/test harness paths (`codex-cli`, `gemini-cli`, `git-*`, `api-*`, `screen-record`, `agent-docs`, `plan-tooling`, `semantic-commit`).
- Workspace gates:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - Coverage gate with `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - Docs checks in final sprint via `--docs-only` plus docs placement strict audit.

## Risks & gotchas
- Shared-helper extraction can unintentionally alter output wording or exit behavior if adapters are bypassed.
- `NO_COLOR`, auth-secret path, and atomic write semantics are contract-sensitive and need explicit parity tests.
- Test helper convergence can introduce hidden ordering dependencies without strict `GlobalStateLock` usage.
- Progress changes can leak into stdout in JSON/machine modes if stderr separation is not strictly enforced.
- Large multi-crate migration spans increase merge-conflict risk; lane ownership and sequencing must be respected.

## Rollback plan
1. Revert by sprint/task cluster (runtime, progress, tests, docs) rather than one monolithic rollback.
2. For runtime regressions, revert consumer migrations first, then shared API additions only if needed.
3. For flaky-test regressions, keep shared helper APIs but roll back specific harness migrations causing instability.
4. Preserve audit artifacts and characterization tests during rollback so reattempt scope remains bounded.
5. After each rollback step, rerun targeted crate tests and required checks before retrying forward.
