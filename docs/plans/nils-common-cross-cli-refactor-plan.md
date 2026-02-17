# Plan: Cross-CLI shared extraction into `nils-common`

## Overview
This plan identifies and extracts the highest-value cross-CLI utility logic into `crates/nils-common` without changing CLI behavior, output text, or exit codes. The primary targets are helpers currently duplicated across multiple binaries: process spawning/PATH lookup, truthy env parsing + `NO_COLOR` gating, shell quote + ANSI stripping, git repo probes/command wrappers, and best-effort clipboard copy. The refactor will be staged with characterization tests first, then module extraction, then per-CLI migration, so parity regressions are caught early.

## Scope
- In scope:
  - Extract generic helpers shared by multiple CLI crates into `crates/nils-common`.
  - Migrate call sites in CLI crates to shared helpers with compatibility adapters when needed.
  - Add/update tests that lock behavior parity before and after migration.
  - Update `nils-common` docs to define module boundaries and usage rules.
- Out of scope:
  - Domain-specific consolidation that belongs in dedicated core crates (for example `api-testing-core`, `nils-term`, `nils-test-support`).
  - CLI feature changes, UX copy changes, or new flags.
  - Large internal redesigns unrelated to shared-helper extraction.

## Assumptions (if any)
1. Behavioral parity is mandatory: output text, warnings, colors, and exit codes must remain unchanged.
2. Adding `nils-common` as a dependency to additional CLI crates is acceptable.
3. If a helper has crate-specific messaging, `nils-common` will expose primitive building blocks and each crate will keep thin message adapters.

## Priority targets (most suitable to extract first)
- `process` primitives:
  - duplicated command lookup/spawn/capture patterns in `crates/fzf-cli/src/util.rs`, `crates/git-cli/src/util.rs`, `crates/git-lock/src/git.rs`, `crates/git-summary/src/git.rs`, `crates/git-scope/src/git_cmd.rs`.
- `env` primitives:
  - duplicated truthy parsing / default semantics in `crates/codex-cli/src/starship/mod.rs`, `crates/codex-cli/src/rate_limits/mod.rs`, `crates/screen-record/src/test_mode.rs`, `crates/screen-record/src/linux/portal.rs`, `crates/git-scope/src/main.rs`, `crates/fzf-cli/src/util.rs`.
- `shell` primitives:
  - duplicated single-quote escaping and ANSI stripping in `crates/git-cli/src/utils.rs`, `crates/image-processing/src/util.rs`, `crates/fzf-cli/src/util.rs`, `crates/codex-cli/src/config.rs`, `crates/git-cli/src/commit.rs`, `crates/git-scope/src/render.rs`.
- `git` primitives:
  - repeated `rev-parse` checks and capture wrappers in `crates/fzf-cli/src/git_*.rs`, `crates/git-lock/src/git.rs`, `crates/git-scope/src/git.rs`, `crates/semantic-commit/src/git.rs`, `crates/plan-tooling/src/repo_root.rs`, `crates/image-processing/src/util.rs`.
- `clipboard` primitive:
  - nearly identical best-effort clipboard write flow in `crates/git-cli/src/clipboard.rs` and `crates/fzf-cli/src/defs/block_preview.rs`.

## Acceptance criteria
- `nils-common` exports focused modules for: `process`, `env`, `shell`, `git`, and `clipboard`.
- Duplicated helper implementations are removed or reduced to thin adapters in target CLI crates.
- CLI behavior remains stable:
  - no intentional output/warning/exit-code changes,
  - existing integration tests continue to pass.
- Required repository checks pass:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`.

## Validation
- `cargo test -p nils-common`
- `cargo test -p fzf-cli -p git-cli -p git-lock -p git-scope -p git-summary -p semantic-commit -p codex-cli -p screen-record -p image-processing`
- `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Sprint 1: Inventory + parity guardrails
**Goal**: Freeze current behavior and define extraction boundaries before code movement.
**Demo/Validation**:
- Command(s):
  - `rg -n "cmd_exists|run_capture|run_output|NO_COLOR|shell_quote|strip_ansi|rev-parse|set_clipboard_best_effort" crates/*/src`
  - `cargo test -p fzf-cli -p git-cli -p git-lock -p git-scope -p git-summary -p semantic-commit -p codex-cli -p screen-record -p image-processing`
- Verify:
  - A reviewed inventory and priority ranking exists.
  - Characterization tests capture pre-refactor behavior for risky helper paths.

**Parallel lanes**:
- Lane A: Task 1.1
- Lane B: Task 1.2
- Lane C: Task 1.3

### Task 1.1: Build cross-CLI duplication inventory with extraction ranking
- **Location**:
  - `docs/plans/nils-common-cross-cli-extraction-inventory.md`
  - `crates/fzf-cli/src/util.rs`
  - `crates/git-cli/src/util.rs`
  - `crates/git-lock/src/git.rs`
  - `crates/codex-cli/src/starship/mod.rs`
  - `crates/screen-record/src/linux/portal.rs`
- **Description**: Create an inventory table that maps duplicated helpers to candidate `nils-common` modules and scores each candidate by reuse breadth, behavioral risk, and migration effort. Mark each helper as `extract`, `adapt`, or `keep local` with rationale.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Inventory includes each priority target and all known duplicate call sites.
  - Every candidate has a decision and rationale (`extract`/`adapt`/`keep local`).
  - The extraction order is explicit and actionable.
- **Validation**:
  - `test -s docs/plans/nils-common-cross-cli-extraction-inventory.md`
  - `rg -n "extract|adapt|keep local" docs/plans/nils-common-cross-cli-extraction-inventory.md`

### Task 1.2: Define `nils-common` module contracts and compatibility rules
- **Location**:
  - `crates/nils-common/src/lib.rs`
  - `crates/nils-common/README.md`
  - `docs/plans/nils-common-cross-cli-extraction-inventory.md`
- **Description**: Define exact public API contracts for new modules (`env`, `shell`, `git`, `clipboard`) plus `process` expansion. Document where crates must keep thin adapters to preserve crate-specific error/warning text.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Proposed public functions are listed with signatures and expected semantics.
  - `process` APIs are explicitly specified with signatures and failure semantics.
  - Adapter rules are documented for message-sensitive call sites.
  - No domain-specific logic is assigned to `nils-common`.
- **Validation**:
  - `rg -n "env|shell|git|clipboard|process" crates/nils-common/README.md`

### Task 1.3: Add characterization tests to lock pre-refactor behavior
- **Location**:
  - `crates/fzf-cli/src/util.rs`
  - `crates/git-cli/src/util.rs`
  - `crates/git-cli/src/clipboard.rs`
  - `crates/git-lock/src/prompt.rs`
  - `crates/git-scope/src/render.rs`
  - `crates/codex-cli/src/starship/render.rs`
  - `crates/screen-record/src/test_mode.rs`
- **Description**: Add/extend tests for truthy env parsing, `NO_COLOR` handling, single-quote escaping, ANSI stripping, clipboard tool priority, and git repo probe semantics so refactor safety is test-gated.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Targeted behavior is covered by deterministic tests.
  - Tests fail if helper semantics drift during migration.
- **Validation**:
  - `cargo test -p fzf-cli -p git-cli -p git-lock -p git-scope -p codex-cli -p screen-record`

## Sprint 2: Extract core generic primitives (`env`/`shell`/`process`)
**Goal**: Land low-risk shared modules first and migrate non-git call sites.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-common`
  - `cargo test -p codex-cli -p fzf-cli -p screen-record -p image-processing`
- Verify:
  - New modules are stable in `nils-common`.
  - Non-git CLIs consume shared primitives with unchanged output behavior.

**Parallel lanes**:
- Lane A: Task 2.1
- Lane B: Task 2.2
- Lane C: Task 2.3
- Lane D: Task 2.4 (after Tasks 2.1-2.3)
- Lane E: Task 2.5 (after Tasks 2.1-2.3)

### Task 2.1: Add `nils-common::env` primitives
- **Location**:
  - `crates/nils-common/src/env.rs`
  - `crates/nils-common/src/lib.rs`
  - `crates/codex-cli/src/starship/mod.rs`
  - `crates/codex-cli/src/rate_limits/mod.rs`
  - `crates/screen-record/src/test_mode.rs`
  - `crates/screen-record/src/linux/portal.rs`
  - `crates/git-scope/src/main.rs`
  - `crates/fzf-cli/src/util.rs`
- **Description**: Implement shared env helpers (for example `is_truthy`, `is_truthy_default`, `get_or_default`, `no_color_enabled`) and migrate matching call sites.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Target call sites no longer implement ad-hoc truthy parsing.
  - `NO_COLOR` checks use a shared helper where semantics are identical.
  - Existing tests keep current behavior intact.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo test -p codex-cli -p fzf-cli -p git-scope -p screen-record`

### Task 2.2: Add `nils-common::shell` primitives
- **Location**:
  - `crates/nils-common/src/shell.rs`
  - `crates/nils-common/src/lib.rs`
  - `crates/git-cli/src/utils.rs`
  - `crates/image-processing/src/util.rs`
  - `crates/fzf-cli/src/util.rs`
  - `crates/codex-cli/src/config.rs`
  - `crates/git-cli/src/commit.rs`
  - `crates/git-scope/src/render.rs`
- **Description**: Implement shared shell helpers for single-quote escaping and ANSI stripping, then replace duplicated implementations while preserving exact user-facing behavior.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Duplicate quote/ANSI helper bodies are removed from target crates or replaced by thin wrappers.
  - Existing command snippets and rendered output remain unchanged.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo test -p git-cli -p git-scope -p fzf-cli -p image-processing -p codex-cli`

### Task 2.3: Expand `nils-common::process` command execution helpers
- **Location**:
  - `crates/nils-common/src/process.rs`
  - `crates/git-cli/src/util.rs`
  - `crates/fzf-cli/src/util.rs`
  - `crates/git-lock/src/git.rs`
  - `crates/git-summary/src/git.rs`
  - `crates/git-scope/src/git_cmd.rs`
- **Description**: Add reusable command execution helpers (spawn/capture/check variants) on top of existing PATH lookup primitives, then migrate compatible call sites with adapter layers where message format must stay crate-specific.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Process helper duplication is reduced across target crates.
  - Failure paths still emit expected crate-level messages.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo test -p git-cli -p fzf-cli -p git-lock -p git-summary -p git-scope`

### Task 2.4: Migrate `codex-cli` and `screen-record` to shared non-git primitives
- **Location**:
  - `crates/codex-cli/src/config.rs`
  - `crates/codex-cli/src/agent/exec.rs`
  - `crates/codex-cli/src/starship/render.rs`
  - `crates/screen-record/src/test_mode.rs`
  - `crates/screen-record/src/linux/portal.rs`
- **Description**: Replace local helper call sites in `codex-cli` and `screen-record` with shared `nils-common` `env`/`shell`/`process` primitives and remove obsolete local helpers in these crates.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - `codex-cli` and `screen-record` compile and tests pass with shared helpers.
  - No changes in CLI output contracts.
- **Validation**:
  - `cargo test -p codex-cli -p screen-record`

### Task 2.5: Migrate `fzf-cli` and `image-processing` to shared non-git primitives
- **Location**:
  - `crates/fzf-cli/src/util.rs`
  - `crates/fzf-cli/src/open.rs`
  - `crates/image-processing/src/util.rs`
  - `crates/image-processing/src/processing.rs`
- **Description**: Replace local non-git helper call sites in `fzf-cli` and `image-processing` with shared `nils-common` primitives while preserving existing behavior and output.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - `fzf-cli` and `image-processing` compile and tests pass with shared helpers.
  - Existing output text, warnings, and fallback behavior remain unchanged.
- **Validation**:
  - `cargo test -p fzf-cli -p image-processing`

## Sprint 3: Extract `git` + `clipboard` primitives and migrate git-family CLIs
**Goal**: Consolidate repeated git/repo probing and clipboard flow with parity-safe adapters.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-common`
  - `cargo test -p git-cli -p git-lock -p git-scope -p git-summary -p semantic-commit -p fzf-cli -p plan-tooling`
- Verify:
  - Git-family CLIs use shared repo probe/command helpers.
  - Clipboard behavior remains stable in both call sites.

**Parallel lanes**:
- Lane A: Task 3.1
- Lane B: Task 3.2
- Lane C: Task 3.3 (after Tasks 3.1-3.2)
- Lane D: Task 3.4 (after Tasks 3.1-3.2)

### Task 3.1: Add `nils-common::git` primitives for repo probes and command wrappers
- **Location**:
  - `crates/nils-common/src/git.rs`
  - `crates/nils-common/src/lib.rs`
  - `crates/semantic-commit/src/git.rs`
  - `crates/git-lock/src/git.rs`
  - `crates/git-scope/src/git.rs`
  - `crates/plan-tooling/src/repo_root.rs`
  - `crates/image-processing/src/util.rs`
- **Description**: Implement shared git helpers for work-tree detection, top-level resolution, and common command execution status/capture paths. Keep crate-specific formatting through adapters.
- **Dependencies**:
  - Task 2.3
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Shared git helpers cover repeated `rev-parse` and basic git-run patterns.
  - Target crates migrate probe logic to shared helpers.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo test -p semantic-commit -p git-lock -p git-scope -p plan-tooling -p image-processing`

### Task 3.2: Add `nils-common::clipboard` best-effort text copy helper
- **Location**:
  - `crates/nils-common/src/clipboard.rs`
  - `crates/nils-common/src/lib.rs`
  - `crates/git-cli/src/clipboard.rs`
  - `crates/fzf-cli/src/defs/block_preview.rs`
- **Description**: Extract shared clipboard fallback flow (tool probing + piping text to stdin) with configurable tool order and crate-level message hooks.
- **Dependencies**:
  - Task 2.3
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Both existing clipboard call sites delegate to shared helper.
  - Tool-priority behavior remains unchanged in tests.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo test -p git-cli -p fzf-cli`

### Task 3.3: Migrate `git-cli`, `git-lock`, and `git-summary` to shared helpers
- **Location**:
  - `crates/git-cli/src/commit_shared.rs`
  - `crates/git-cli/src/reset.rs`
  - `crates/git-cli/src/ci.rs`
  - `crates/git-lock/src/git.rs`
  - `crates/git-summary/src/git.rs`
- **Description**: Replace duplicated git command/repo-check logic in these three crates with shared helpers, keeping wrappers where each crate needs specific context/error text.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Duplicated git helper implementations are removed or minimized in the three target crates.
  - Existing CLI behavior and tests remain stable for these crates.
- **Validation**:
  - `cargo test -p git-cli -p git-lock -p git-summary`

### Task 3.4: Migrate `git-scope`, `fzf-cli`, and `semantic-commit` to shared helpers
- **Location**:
  - `crates/git-scope/src/git_cmd.rs`
  - `crates/git-scope/src/git.rs`
  - `crates/fzf-cli/src/git_branch.rs`
  - `crates/fzf-cli/src/git_tag.rs`
  - `crates/fzf-cli/src/git_checkout.rs`
  - `crates/fzf-cli/src/git_commit.rs`
  - `crates/fzf-cli/src/git_status.rs`
  - `crates/semantic-commit/src/git.rs`
- **Description**: Migrate remaining git-oriented crates to shared `git`/`process`/`clipboard` helpers with parity-safe adapters for crate-specific output behavior.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Duplicated git helper logic is reduced in `git-scope`, `fzf-cli`, and `semantic-commit`.
  - Existing output contracts and exit-code behavior remain unchanged.
- **Validation**:
  - `cargo test -p git-scope -p fzf-cli -p semantic-commit`

## Sprint 4: Cleanup, docs, and final quality gates
**Goal**: Finish cleanup, document boundaries, and run full repo checks.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify:
  - Refactor is complete and stable under required checks.
  - `nils-common` boundaries are documented for future contributors.

**Parallel lanes**:
- Lane A: Task 4.1
- Lane B: Task 4.2
- Lane C: Task 4.3 (after Tasks 4.1-4.2)
- Lane D: Task 4.4 (after Tasks 4.1-4.3)

### Task 4.1: Remove obsolete local helper implementations
- **Location**:
  - `crates/git-cli/src/util.rs`
  - `crates/git-cli/src/clipboard.rs`
  - `crates/fzf-cli/src/util.rs`
  - `crates/git-lock/src/git.rs`
  - `crates/git-summary/src/git.rs`
  - `crates/git-scope/src/git_cmd.rs`
  - `crates/codex-cli/src/starship/mod.rs`
  - `crates/screen-record/src/linux/portal.rs`
- **Description**: Remove dead local helper functions after migrations complete, while preserving thin adapters where crate-specific messaging is required.
- **Dependencies**:
  - Task 2.4
  - Task 2.5
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - No stale helper duplicates remain in migrated source modules.
  - Only parity-preserving adapters remain where intentionally required.
- **Validation**:
  - `rg -n "fn (env_truthy|env_is_true|shell_quote|shell_escape|strip_ansi|set_clipboard_best_effort|is_git_repo|run_capture)" crates/*/src`

### Task 4.2: Normalize `Cargo.toml` dependencies after helper migration
- **Location**:
  - `crates/git-cli/Cargo.toml`
  - `crates/git-lock/Cargo.toml`
  - `crates/git-scope/Cargo.toml`
  - `crates/git-summary/Cargo.toml`
  - `crates/fzf-cli/Cargo.toml`
  - `crates/codex-cli/Cargo.toml`
  - `crates/semantic-commit/Cargo.toml`
  - `crates/plan-tooling/Cargo.toml`
  - `crates/image-processing/Cargo.toml`
- **Description**: Update crate dependencies to reflect shared-helper usage (adding/removing `nils-common` where needed) and keep dependency declarations minimal and explicit.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 5
- **Acceptance criteria**:
  - All migrated crates have correct and minimal dependency entries.
  - Workspace compiles successfully with normalized dependency graph.
- **Validation**:
  - `cargo check --workspace`

### Task 4.3: Update shared-helper documentation and contributor guidance
- **Location**:
  - `crates/nils-common/README.md`
  - `README.md`
  - `AGENTS.md`
- **Description**: Document what belongs in `nils-common`, what should stay crate-local, and migration conventions for preserving output parity when introducing shared helpers.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `nils-common` README includes module purpose, API examples, and non-goals.
  - Repository docs point contributors to the shared-helper policy.
- **Validation**:
  - `rg -n "nils-common|shared helper|parity" crates/nils-common/README.md README.md AGENTS.md`

### Task 4.4: Run final mandatory checks and parity smoke tests
- **Location**:
  - `DEVELOPMENT.md`
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- **Description**: Execute required lint/test gates and targeted CLI smoke checks to confirm no behavior regressions after extraction.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
  - Task 4.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Mandatory checks complete successfully.
  - No known behavior regression is introduced by shared-helper migration.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Testing Strategy
- Unit:
  - Add unit tests per new `nils-common` module (`env`, `shell`, `process`, `git`, `clipboard`) covering edge cases and platform-sensitive behavior.
- Integration:
  - Keep existing crate integration tests as parity gates; extend only where helper behavior was implicit.
- E2E/manual:
  - Run representative CLI commands for git-family tools plus `codex-cli`, `fzf-cli`, `screen-record`, and `image-processing` to verify unchanged stdout/stderr contracts.

## Risks & gotchas
- Message drift risk:
  - Generic helpers may accidentally alter crate-specific error/warning text; keep adapters where messages are contract-sensitive.
- Over-generalization risk:
  - Pushing domain-specific rules into `nils-common` will create coupling; enforce `extract/adapt/keep local` decisions from the inventory.
- Platform behavior risk:
  - Clipboard and executable lookup differ across Unix variants; preserve existing fallback order and test with stubs.
- Refactor blast radius:
  - Migrating too many crates in one step can hide regressions; follow sprinted rollout with explicit dependency gates.

## Rollback plan
- Revert migration in reverse order:
  1. Revert Sprint 4 cleanup/docs if checks fail.
  2. Revert Sprint 3 call-site migrations while keeping new `nils-common` modules unused.
  3. Revert Sprint 2 non-git migrations if output drift is detected.
  4. Keep Sprint 1 characterization tests to isolate the regression cause.
- Operational fallback:
  - Temporarily restore per-crate local helper wrappers (delegating back to prior logic) while preserving tests, then retry migration one module at a time.
