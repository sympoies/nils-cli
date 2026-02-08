# Plan: agent-docs worktree policy fallback support

## Overview
This plan adds first-class linked-worktree support to `agent-docs` so required policy documents can still resolve when they are intentionally not committed and only exist in the primary worktree. The core behavior is: resolve from the current project path first, then fallback to the primary worktree for equivalent project-scope files when running inside a linked worktree. The plan keeps strict mode meaningful, preserves non-worktree behavior, and adds explicit output metadata so users can see when fallback was used.

## Scope
- In scope:
  - Detect linked-worktree metadata (`is_worktree`, `git_common_dir`, `primary_worktree_path`) during root resolution.
  - Add deterministic fallback for project-scope required docs (`startup`, `project-dev`, and required `AGENT_DOCS.toml` project entries).
  - Surface fallback provenance in `resolve`/`baseline` output.
  - Add integration tests that reproduce "doc exists only in primary worktree".
  - Improve scaffold behavior so project baseline docs can be copied from primary worktree content instead of generic templates when available.
- Out of scope:
  - Remote/shared sync of policy files outside local Git metadata.
  - Cross-repository fallback (only current Git repository worktrees are considered).
  - Automatic commits/staging for copied baseline docs.

## Assumptions (if any)
1. Teams using worktrees may keep `AGENTS.md` (and similar policy docs) untracked by design.
2. In standard non-bare repositories, `git rev-parse --git-common-dir` points to `<primary>/.git` and can be used to infer the primary worktree root.
3. `git` is available at runtime (already required by existing `PROJECT_PATH` fallback logic).
4. If multiple linked worktrees exist, the primary worktree is the authoritative fallback source.

## Success Criteria
1. `agent-docs resolve --context startup --strict --format checklist` passes in a linked worktree when `AGENTS.md` is missing locally but present in the primary worktree.
2. `agent-docs baseline --check --target project --strict` reports no missing required project docs under the same scenario.
3. Output clearly discloses fallback usage (source + fallback path) to avoid hidden behavior.
4. Existing non-worktree behavior and tests remain unchanged.

## Sprint 1: Worktree detection + contract
**Goal**: Introduce explicit linked-worktree metadata and freeze fallback precedence/compatibility contract before resolver changes.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agent-docs env_paths`
  - `cargo run -q -p agent-docs -- resolve --context startup --format json`
- Verify:
  - Runtime metadata can distinguish non-worktree vs linked-worktree execution.
  - Contract docs describe exact precedence and compatibility behavior.

### Task 1.1: Document worktree fallback contract and precedence
- **Location**:
  - `crates/agent-docs/README.md`
- **Description**: Add a new "Worktree fallback" section defining precedence for project-scope docs: local override/default first, then primary-worktree equivalent path; include strict-mode semantics and an explicit compatibility note that non-worktree repos are unchanged.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - README includes a deterministic fallback order for `startup` and `project-dev` project-scope docs.
  - README documents how fallback appears in output and how to disable fallback if needed.
- **Validation**:
  - `rg -n "^## Worktree fallback$|fallback order|local-only|linked worktree" crates/agent-docs/README.md`
  - `rg -n "resolve --context startup --strict|baseline --check --target project --strict" crates/agent-docs/README.md`

### Task 1.2: Extend root resolution with linked-worktree metadata
- **Location**:
  - `crates/agent-docs/src/env.rs`
  - `crates/agent-docs/src/model.rs`
  - `crates/agent-docs/tests/env_paths.rs`
- **Description**: Extend resolved environment/root model to capture linked-worktree metadata (`is_linked_worktree`, `git_common_dir`, `primary_worktree_path`). Implement detection using `git rev-parse --absolute-git-dir` + `--git-common-dir`, with robust fallback to `None` when unavailable.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Metadata is populated for linked worktrees and absent for regular repos/non-git directories.
  - Detection tolerates command failures without panicking and preserves existing root resolution behavior.
- **Validation**:
  - `cargo test -p agent-docs env_paths`

### Task 1.3: Add explicit fallback mode toggle
- **Location**:
  - `crates/agent-docs/src/cli.rs`
  - `crates/agent-docs/src/model.rs`
  - `crates/agent-docs/src/lib.rs`
- **Description**: Add a global mode switch for project fallback behavior (`auto` default, `local-only` opt-out) so teams can keep deterministic local-only enforcement if desired.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Default mode enables worktree fallback when metadata is available.
  - `local-only` mode disables fallback and restores current strict local behavior.
  - `--help` documents the new switch clearly.
- **Validation**:
  - `cargo run -q -p agent-docs -- --help | rg -n "worktree|local-only|auto"`
  - `cargo test -p agent-docs`

## Sprint 2: Resolver + baseline fallback implementation
**Goal**: Apply fallback resolution to project-scope required docs and make fallback provenance explicit in machine/human outputs.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agent-docs resolve_builtin baseline`
  - `cargo run -q -p agent-docs -- resolve --context startup --format checklist --strict`
- Verify:
  - Strict resolve/baseline pass when required files exist in primary worktree fallback path.
  - Output identifies fallback source path.

### Task 2.1: Extend document model/output for fallback provenance
- **Location**:
  - `crates/agent-docs/src/model.rs`
  - `crates/agent-docs/src/output.rs`
- **Description**: Add explicit provenance fields for resolved docs (e.g., `resolved_from` or `source_path`) and a new source type for worktree fallback to avoid ambiguous "present" status.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Output can represent both logical target path and actual fallback source path.
  - Existing text/checklist/json output remains parseable; new fields are additive.
- **Validation**:
  - `cargo test -p agent-docs resolve_builtin`

### Task 2.2: Implement project-scope fallback chain in resolver
- **Location**:
  - `crates/agent-docs/src/resolver.rs`
  - `crates/agent-docs/src/paths.rs`
- **Description**: Implement fallback lookup chain for project-scope docs when in `auto` mode: (1) current worktree path, (2) primary worktree equivalent path. Apply to startup project policy (`AGENTS.override.md` then `AGENTS.md`), built-in `project-dev` docs, and required project-scope extension docs (including `AGENT_DOCS.toml` required entries whose local file is missing but primary equivalent exists).
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - `startup` project policy resolves `AGENTS.override.md`/`AGENTS.md` from primary worktree when missing locally.
  - `project-dev` required docs and required extension docs can resolve via equivalent primary worktree paths.
  - Required project-scope `AGENT_DOCS.toml` entries are considered present when their equivalent primary-worktree path exists.
  - Home-scope docs never use project worktree fallback.
- **Validation**:
  - `cargo test -p agent-docs resolve_builtin`
  - `cargo test -p agent-docs resolve_toml`
  - `cargo test -p agent-docs worktree_fallback -- --nocapture`

### Task 2.3: Align baseline check with resolver fallback semantics
- **Location**:
  - `crates/agent-docs/src/commands/baseline.rs`
  - `crates/agent-docs/src/output.rs`
- **Description**: Reuse the same fallback chain in baseline checks so strict baseline behaves consistently with strict resolve. Include fallback provenance in baseline text/json output.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - `baseline --strict` no longer fails for project-scope docs when fallback source exists.
  - Suggested actions still appear only when neither local nor fallback source exists.
- **Validation**:
  - `cargo test -p agent-docs baseline`

### Task 2.4: Add linked-worktree integration tests (real Git worktree)
- **Location**:
  - `crates/agent-docs/tests/worktree_fallback.rs`
  - `crates/agent-docs/tests/common.rs`
- **Description**: Add end-to-end tests that create a temp repo, add a linked worktree, place `AGENTS.md` only in primary worktree, then execute `agent-docs` from linked worktree to validate strict resolve/baseline behavior under `auto` and `local-only` modes.
- **Dependencies**:
  - Task 1.2
  - Task 2.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Test reproduces current failure mode and verifies fallback fix.
  - Tests cover both startup policy and at least one project-dev required doc/extension path.
- **Validation**:
  - `cargo test -p agent-docs worktree_fallback -- --nocapture`

## Sprint 3: Scaffold UX + migration safety
**Goal**: Ensure remediation commands produce expected project-specific content in worktrees and reduce accidental template overwrite drift.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agent-docs scaffold_baseline`
  - `agent-docs scaffold-baseline --target project --missing-only --dry-run --format text`
- Verify:
  - Scaffold reports when content is sourced from primary worktree fallback.
  - Existing template-based behavior remains available when no fallback source exists.

### Task 3.1: Teach scaffold-baseline to copy from primary fallback when available
- **Location**:
  - `crates/agent-docs/src/commands/scaffold_baseline.rs`
  - `crates/agent-docs/tests/scaffold_baseline.rs`
- **Description**: For missing project baseline docs in linked worktrees, prefer copying actual file contents from primary worktree equivalent paths (if present) before falling back to default templates. Keep `--force`/`--missing-only` semantics unchanged.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - `--missing-only` creates missing project docs from primary content when available.
  - Action reason/output clearly states "created from primary worktree fallback".
  - No behavior change for non-worktree repos.
- **Validation**:
  - `cargo test -p agent-docs scaffold_baseline`

### Task 3.2: Add migration guidance and troubleshooting
- **Location**:
  - `crates/agent-docs/README.md`
  - `docs/plans/agent-docs-worktree-policy-fallback-plan.md`
- **Description**: Document operator guidance for teams currently relying on local-only behavior: when to use `local-only`, how to hydrate missing docs into worktrees, and how to interpret fallback paths in checklist/baseline output.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 3
- **Acceptance criteria**:
  - README includes migration examples for both fallback-enabled and local-only flows.
  - Troubleshooting section covers detached worktree + missing primary file scenarios.
- **Validation**:
  - `rg -n "^## Migration|^## Troubleshooting|local-only|fallback-enabled|hydrate" crates/agent-docs/README.md`
  - `rg -n "detached|primary worktree|local-only" crates/agent-docs/README.md`

## Sprint 4: Regression hardening + rollout
**Goal**: Verify end-to-end safety across worktree/non-worktree paths and keep release risk low.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agent-docs`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - All existing checks pass.
  - No regressions in CLI output contracts consumed by automation.

### Task 4.1: Add compatibility regression tests for non-worktree behavior
- **Location**:
  - `crates/agent-docs/tests/resolve_builtin.rs`
  - `crates/agent-docs/tests/baseline.rs`
- **Description**: Add assertions that behavior in normal repos remains unchanged (source/status/order) and that `local-only` mode preserves current strict failure semantics when project docs are missing locally.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Existing snapshots/expectations remain stable or are intentionally updated with documented rationale.
  - `local-only` mode reproduces pre-feature missing-doc failures in linked worktree tests.
- **Validation**:
  - `cargo test -p agent-docs resolve_builtin baseline`

### Task 4.2: Final contract pass for release readiness
- **Location**:
  - `crates/agent-docs/README.md`
  - `BINARY_DEPENDENCIES.md`
- **Description**: Ensure CLI contract docs, examples, and dependency expectations align with the new worktree behavior and flags.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 2
- **Acceptance criteria**:
  - README command examples compile with implemented flags/outputs.
  - No conflicting statements about project-path-only resolution remain.
- **Validation**:
  - `rg -n "PROJECT_PATH|worktree|fallback|local-only" crates/agent-docs/README.md BINARY_DEPENDENCIES.md`

## Dependency and Parallelization Map
- Critical path:
  - Task 1.1 -> Task 1.2 -> Task 1.3 -> Task 2.1 -> Task 2.2 -> Task 2.3 -> Task 2.4 -> Task 4.1 -> Task 4.2
- Parallelizable groups:
  - Task 2.4 can begin once Task 2.3 API shape is stable; test fixture scaffolding can start in parallel with Task 3.1.
  - Task 3.2 can run in parallel with Task 4.1 once fallback output strings are finalized.

## Testing Strategy
- Unit:
  - Root/worktree metadata detection in `env` tests.
  - Resolver candidate-chain tests for startup/project-dev and extension docs.
  - Baseline fallback-specific status/source assertions.
- Integration:
  - Real `git worktree` fixture tests for `resolve --strict` and `baseline --strict`.
  - Scaffold tests verifying copy-from-primary fallback and fallback-to-template behavior.
- E2E/manual:
  - In a linked worktree where `AGENTS.md` exists only in primary, run:
    - `agent-docs resolve --context startup --strict --format checklist`
    - `agent-docs baseline --check --target project --strict --format text`
    - `agent-docs scaffold-baseline --target project --missing-only --dry-run --format text`

## Risks & gotchas
- Fallback can hide local drift if users expect every worktree to have explicit copies. Mitigation: output provenance + `local-only` mode.
- Inferring primary worktree path from Git metadata can fail in unusual repository layouts. Mitigation: fail closed to local-only behavior when metadata is incomplete.
- Output schema changes can break downstream parsers. Mitigation: additive fields only and compatibility tests for text/checklist formats.
- Worktree tests can be flaky on constrained CI environments. Mitigation: keep fixture setup deterministic and isolate `git` command assumptions.

## Rollback plan
- Add an emergency release toggle that defaults to `local-only` (or hard-disable fallback path) without removing new CLI fields.
- Revert resolver/baseline fallback logic to local-path-only while keeping metadata detection inert.
- Keep integration tests but mark fallback-specific ones ignored until re-enabled; retain non-worktree regression coverage.
- If scaffold copy-from-primary introduces risk, revert only `scaffold_baseline` changes and keep read-only resolve/baseline fallback.
