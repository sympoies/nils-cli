# Plan: Nils Common / Nils Test Support Adoption for Maintainability

## Overview
This plan inventories and fixes places in the workspace that still reimplement primitives that should live in `nils-common` or `nils-test-support`, with a strong bias toward behavior-preserving migrations.  
The execution strategy is multi-PR by low-overlap file clusters: start with an explicit manifest + characterization coverage, then land small/medium adoptions, and only then do higher-risk shared helper extraction.  
Two types of work are included: (1) adopt existing shared helpers, and (2) add missing shared primitives (in `nils-common` / `nils-test-support`) when multiple crates/tests are already duplicating the same logic.  
Success means every candidate in the manifest is either migrated to shared helpers or explicitly marked `keep-local` with a parity/contract rationale.

## Scope
- In scope: runtime helper migrations to `nils-common` (`env`, `process`, `git`, `fs`, `provider_runtime`-adjacent path helpers).
- In scope: test helper migrations to `nils-test-support` (`EnvGuard`, `GlobalStateLock`, `prepend_path`, `StubBinDir`, `bin`, `cmd`, `git`, `fs`).
- In scope: minimal shared helper extensions required to remove repeated local implementations while preserving CLI output/error/exit-code contracts.
- Out of scope: new end-user features, CLI UX copy redesign, or JSON schema changes unrelated to helper reuse.
- Out of scope: broad refactors that mix helper migration with unrelated behavior changes.

## Assumptions (if any)
1. Each task below is intended to land as an independent PR unless a dependency note explicitly recommends bundling.
2. Characterization tests must be added/updated before any migration that can affect observable output, warnings, or exit codes.
3. `nils-common` may grow new domain-neutral APIs when two or more crates already duplicate the same primitive (for example atomic write/timestamp helpers).
4. Some current behavior may be intentionally stricter than shared defaults (notably env-only secret directory resolution in `codex-cli auth save/remove`); those cases require explicit keep/change decisions.

## Candidate Inventory (Current Audit Baseline)
- `nils-common` adoption/extraction candidates:
  - `crates/memo-cli/src/output/text.rs` (`NO_COLOR` parsing + unit test env mutation)
  - `crates/gemini-cli/src/agent/commit.rs` (manual `PATH` scan / `command_exists`)
  - `crates/codex-cli/src/auth/save.rs`, `crates/codex-cli/src/auth/remove.rs` (duplicated env-only secret-dir resolver)
  - `crates/git-lock/src/diff.rs`, `crates/git-lock/src/tag.rs` (manual `git` process invocations)
  - `crates/git-scope/src/print.rs` (manual `git` invocations for object/file reads)
  - `crates/plan-tooling/src/validate.rs` (`git ls-files` shell-out wrapper)
  - `crates/semantic-commit/src/commit.rs`, `crates/semantic-commit/src/staged_context.rs` (manual `git` command wrappers)
  - `crates/codex-cli/src/fs.rs`, `crates/gemini-cli/src/fs.rs`, `crates/gemini-cli/src/auth/mod.rs` (duplicated atomic write/timestamp/hash helpers)
- `nils-test-support` adoption candidates:
  - `crates/git-cli/src/commit.rs` test module (local `EnvGuard`)
  - `crates/gemini-cli/src/auth/login.rs`, `crates/gemini-cli/src/auth/auto_refresh.rs` test modules (local env guards, path prepend, script writers, temp dirs)
  - `crates/gemini-cli/src/agent/commit.rs` test module (local env guard + temp dir + executable writer)
  - `crates/gemini-cli/tests/paths.rs`, `crates/gemini-cli/tests/prompts.rs` (custom `EnvVarGuard` + custom env lock + custom temp dir)
  - `crates/gemini-cli/tests/agent_prompt.rs` (custom temp dir + executable writer)
  - `crates/gemini-cli/tests/auth_refresh.rs` (manual stub writer + manual `PATH` prepend helper)
  - `crates/codex-cli/tests/agent_commit.rs`, `crates/gemini-cli/tests/agent_commit_fallback.rs` (manual `git` setup/commands)
  - `crates/agent-docs/tests/env_paths.rs` (manual `git` repo/worktree setup sequence)
  - `crates/git-scope/tests/help_outside_repo.rs`, `crates/git-scope/tests/edge_cases.rs` (manual binary resolution / allow-fail runner)
  - `crates/api-grpc/tests/integration.rs`, `crates/api-test/tests/grpc_integration.rs`, `crates/api-testing-core/tests/suite_runner_grpc_matrix.rs` (manual executable stub writes and env mutation)
  - `crates/screen-record/tests/linux_request_permission.rs` (manual executable stub writer)
  - `crates/codex-cli/tests/agent_templates.rs`, `crates/codex-cli/tests/auth_json_contract.rs`, `crates/gemini-cli/tests/agent_templates.rs`, `crates/fzf-cli/tests/open_and_file.rs`, `crates/fzf-cli/tests/git_commands.rs`, `crates/fzf-cli/tests/git_commit.rs` (manual `PATH` prepend string helpers)

## Sprint 1: Exhaustive Manifest and PR Slicing
**Goal**: Lock an exact, machine-checkable adoption manifest before implementation PRs start.
**Demo/Validation**:
- Command(s): `plan-tooling validate --file docs/plans/nils-common-nils-test-support-maintainability-plan.md`
- Verify: plan parses cleanly and the manifest-generation workflow below can produce a complete candidate list with status tracking.

### Task 1.1: Generate machine-readable helper adoption manifest
- **Location**:
  - `scripts/dev/shared-helper-adoption-audit.sh`
  - `$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv`
  - `$AGENT_HOME/out/nils-cli-shared-helper-adoption/summary.md`
- **Description**: Add a repeatable audit script that scans for known duplication patterns (custom env guards, manual `CARGO_BIN_EXE_*` resolution, manual git test setup, manual `PATH` prepend helpers, manual executable `chmod`, local `NO_COLOR` checks, manual `command_exists` path scans) and records each candidate with category, target shared helper, status, and owning PR task.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Manifest output includes every file in the current audit baseline section.
  - Each manifest row includes `category`, `helper_target`, `path`, `status`, and `task_id`.
  - Script is safe to rerun and writes artifacts under `$AGENT_HOME/out/...`.
- **Validation**:
  - `mkdir -p "$AGENT_HOME/out/nils-cli-shared-helper-adoption"`
  - `bash scripts/dev/shared-helper-adoption-audit.sh --format tsv --out "$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv"`
  - `test -s "$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv"`
  - `for p in crates/memo-cli/src/output/text.rs crates/gemini-cli/src/agent/commit.rs crates/agent-docs/tests/env_paths.rs; do rg -n --fixed-strings "$p" "$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv"; done`

### Task 1.2: Codify keep-local vs shared-helper decision rules for this migration
- **Location**:
  - `$AGENT_HOME/out/nils-cli-shared-helper-adoption/decision-matrix.md`
  - `docs/plans/nils-common-nils-test-support-maintainability-plan.md`
- **Description**: Create a migration decision matrix that classifies candidates into `adopt existing helper`, `extend shared helper then adopt`, or `keep local`, with parity constraints derived from `AGENTS.md` shared helper policy.
- **Dependencies**:
  - `Task 1.1`
- **Complexity**: 4
- **Acceptance criteria**:
  - Decision matrix explicitly covers runtime UX/exit semantics and test determinism constraints.
  - Every manifest category maps to one of the three migration outcomes.
  - High-risk categories (auth secret-dir resolution, atomic writes) are flagged for characterization-first handling.
- **Validation**:
  - `test -s "$AGENT_HOME/out/nils-cli-shared-helper-adoption/decision-matrix.md"`
  - `rg -n 'adopt existing helper|extend shared helper|keep local' "$AGENT_HOME/out/nils-cli-shared-helper-adoption/decision-matrix.md"`
  - `rg -n 'parity|exit code|warning|message' "$AGENT_HOME/out/nils-cli-shared-helper-adoption/decision-matrix.md"`
  - `rg -n 'characterization-first|high-risk|Task 2\\.5|Task 3\\.' "$AGENT_HOME/out/nils-cli-shared-helper-adoption/decision-matrix.md"`

### Task 1.3: Finalize PR batching matrix and dependency graph
- **Location**:
  - `$AGENT_HOME/out/nils-cli-shared-helper-adoption/pr-batches.md`
  - `docs/plans/nils-common-nils-test-support-maintainability-plan.md`
- **Description**: Assign every manifest row to a concrete PR slice (task ID) with conflict notes so multiple PRs can proceed in parallel without file overlap.
- **Dependencies**:
  - `Task 1.1`
  - `Task 1.2`
- **Complexity**: 3
- **Acceptance criteria**:
  - Every manifest row has an assigned `task_id`.
  - No two parallelized tasks claim the same file without an explicit sequencing note.
  - PR batching document includes expected check commands per batch.
- **Validation**:
  - `test -s "$AGENT_HOME/out/nils-cli-shared-helper-adoption/pr-batches.md"`
  - `rg -n 'Task 2\\.|Task 3\\.|Task 4\\.|Task 5\\.' "$AGENT_HOME/out/nils-cli-shared-helper-adoption/pr-batches.md"`
  - `awk -F '\t' 'NR>1 && $5==\"\" {print $0}' "$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv" | wc -l | rg '^0$'`

## Sprint 2: Existing `nils-common` Helper Adoption (Low/Medium Risk)
**Goal**: Replace obvious local reimplementations with existing `nils-common` helpers while preserving behavior.
**Demo/Validation**:
- Command(s): `cargo test -p nils-memo-cli && cargo test -p gemini-cli agent_commit && cargo test -p git-lock && cargo test -p git-scope`
- Verify: runtime behavior remains stable and low-risk helper adoptions are complete.

### Task 2.1: Migrate `memo-cli` NO_COLOR handling to `nils_common::env`
- **Location**:
  - `crates/memo-cli/Cargo.toml`
  - `crates/memo-cli/src/output/text.rs`
- **Description**: Replace local `NO_COLOR` parsing in `memo-cli` text rendering with `nils_common::env` helper usage (with a local adapter only if current empty-string semantics must be preserved), and replace unsafe unit-test env mutation with `nils_test_support::{EnvGuard, GlobalStateLock}` in the same file.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 4
- **Acceptance criteria**:
  - `memo-cli` depends on `nils-common`.
  - `color_enabled()` no longer reads `NO_COLOR` directly without a shared helper path.
  - Unit tests avoid raw `unsafe { std::env::set_var/remove_var }`.
  - Existing `NO_COLOR` behavior is characterized and preserved (or intentionally changed with explicit test update).
- **Validation**:
  - `cargo test -p nils-memo-cli text_output_respects_no_color`
  - `cargo test -p nils-memo-cli style_helpers_cover_color_and_no_color_modes`
  - `rg -n 'nils-common' crates/memo-cli/Cargo.toml`
  - `rg -n 'nils_common::env|shared_env' crates/memo-cli/src/output/text.rs`
  - `if rg -n 'unsafe \\{ std::env::(set_var|remove_var)' crates/memo-cli/src/output/text.rs; then echo 'unexpected raw env mutation remains'; exit 1; fi`

### Task 2.2: Migrate `gemini-cli` agent commit command probing to `nils_common::process` (and clean test helpers in same file)
- **Location**:
  - `crates/gemini-cli/src/agent/commit.rs`
- **Description**: Replace the local `command_exists`/manual `PATH` scan implementation with `nils_common::process::cmd_exists`, remove redundant local executable-probe helpers when possible, and migrate the file’s test module env/temp/executable helpers to `nils_test_support` equivalents.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Runtime `command_exists` path probing delegates to `nils-common`.
  - File-local test helpers do not reimplement env guards or executable writes already provided by `nils-test-support`.
  - `command_exists` test semantics (including executable-bit behavior on Unix) remain covered.
- **Validation**:
  - `cargo test -p gemini-cli command_exists_checks_executable_bit`
  - `cargo test -p gemini-cli agent_commit_fallback`
  - `rg -n 'process::cmd_exists|nils_common::process' crates/gemini-cli/src/agent/commit.rs`

### Task 2.3: Normalize low-level `git` process wrappers in `git-lock` and `git-scope` to `nils_common::git` / `nils_common::process`
- **Location**:
  - `crates/git-lock/src/diff.rs`
  - `crates/git-lock/src/tag.rs`
  - `crates/git-scope/src/print.rs`
- **Description**: Replace behavior-neutral `Command::new(\"git\")` call sites with shared `nils_common::git` / `nils_common::process` wrappers where the command composition and UX text can remain crate-local but subprocess plumbing becomes shared.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 7
- **Acceptance criteria**:
  - Shared wrappers are used for low-level execution at the identified sites.
  - Existing output text, warnings, and exit-code behavior remain unchanged.
  - Any call sites that must remain direct `Command::new` are documented with rationale in the PR.
- **Validation**:
  - `cargo test -p git-lock`
  - `cargo test -p git-scope`
  - `rg -n 'Command::new\\(\"git\"\\)' crates/git-lock/src/diff.rs crates/git-lock/src/tag.rs crates/git-scope/src/print.rs`

### Task 2.4: Normalize low-level `git` process wrappers in `plan-tooling` and `semantic-commit`
- **Location**:
  - `crates/plan-tooling/src/validate.rs`
  - `crates/semantic-commit/src/commit.rs`
  - `crates/semantic-commit/src/staged_context.rs`
- **Description**: Refactor manual `git` subprocess plumbing to reuse shared wrappers while preserving `GIT_PAGER`/`PAGER` environment behavior and existing error text.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Shared wrappers are used at the identified low-level subprocess sites, or exceptions are documented with parity rationale.
  - `semantic-commit` summary/staged-context outputs and errors remain stable.
  - `plan-tooling validate` default plan discovery still behaves identically inside/outside git repos.
- **Validation**:
  - `cargo test -p plan-tooling validate`
  - `cargo test -p semantic-commit`
  - `rg -n 'Command::new\\(\"git\"\\)' crates/plan-tooling/src/validate.rs crates/semantic-commit/src/commit.rs crates/semantic-commit/src/staged_context.rs`

### Task 2.5: Remove duplicated env-only secret-dir resolver in `codex-cli` auth save/remove via shared path helper strategy
- **Location**:
  - `crates/codex-cli/src/auth/save.rs`
  - `crates/codex-cli/src/auth/remove.rs`
  - `crates/codex-cli/tests/auth_save.rs`
  - `crates/codex-cli/tests/auth_remove.rs`
  - `crates/codex-cli/src/paths.rs`
  - `crates/nils-common/src/provider_runtime/paths.rs`
- **Description**: Eliminate duplicated `resolve_secret_dir_from_env()` implementations in `codex-cli` auth save/remove by adopting a shared path helper approach. If `crate::paths::resolve_secret_dir()` changes behavior too much, add an explicit env-only helper in a shared layer and keep current UX/error contracts.
- **Dependencies**:
  - `Task 1.2`
  - `Task 1.3`
- **Complexity**: 7
- **Acceptance criteria**:
  - `codex-cli` no longer has duplicated `resolve_secret_dir_from_env()` implementations in both files.
  - Behavior for missing secret-dir configuration is explicitly characterized and preserved (or intentionally changed with test updates and rationale).
  - Shared helper ownership is clear (`codex-cli paths` thin adapter or `nils-common` provider-runtime helper).
- **Validation**:
  - `cargo test -p codex-cli auth_save`
  - `cargo test -p codex-cli auth_remove`
  - `rg -n 'fn resolve_secret_dir_from_env' crates/codex-cli/src/auth/save.rs crates/codex-cli/src/auth/remove.rs`

## Sprint 3: `nils-common` File I/O Primitive Extraction and Adoption (Higher Risk)
**Goal**: Extract repeated atomic-write/timestamp/hash primitives into shared code, then migrate codex/gemini callers in bounded steps.
**Demo/Validation**:
- Command(s): `cargo test -p nils-common fs && cargo test -p codex-cli auth && cargo test -p gemini-cli auth`
- Verify: shared file I/O helpers cover codex/gemini auth/rate-limit workflows without contract regressions.

### Task 3.1: Add shared atomic-write/timestamp/hash primitives to `nils-common`
- **Location**:
  - `crates/nils-common/src/fs.rs`
  - `crates/nils-common/README.md`
- **Description**: Extend `nils-common::fs` with domain-neutral primitives that codex/gemini currently duplicate (`write_atomic` temp-file pattern, timestamp write/remove helper, file hash helper if still duplicated after audit), including parity-focused tests for permissions and overwrite semantics.
- **Dependencies**:
  - `Task 1.2`
  - `Task 1.3`
- **Complexity**: 9
- **Acceptance criteria**:
  - New `nils-common::fs` APIs are domain-neutral and return structured errors without CLI-specific messaging.
  - Unit tests cover overwrite behavior, temp-file collision retries, timestamp trimming/removal, and Unix permission paths.
  - `nils-common` docs describe migration constraints and non-goals.
- **Validation**:
  - `cargo test -p nils-common fs`
  - `rg -n 'write_atomic|write_timestamp|sha256' crates/nils-common/src/fs.rs`

### Task 3.2: Migrate `codex-cli` and `gemini-cli` top-level fs modules to `nils-common::fs` primitives
- **Location**:
  - `crates/codex-cli/src/fs.rs`
  - `crates/gemini-cli/src/fs.rs`
  - `crates/codex-cli/src/rate_limits/writeback.rs`
  - `crates/gemini-cli/src/rate_limits/mod.rs`
- **Description**: Convert codex/gemini local fs modules into thin adapters over `nils-common::fs` for shared primitive behavior while preserving crate-local error-context formatting and constants.
- **Dependencies**:
  - `Task 3.1`
- **Complexity**: 8
- **Acceptance criteria**:
  - Shared primitive logic no longer exists independently in both `codex-cli/src/fs.rs` and `gemini-cli/src/fs.rs`.
  - Crate-local adapters preserve existing return types and error context strings where required.
  - Rate-limit cache/writeback call sites continue passing existing tests unchanged.
- **Validation**:
  - `cargo test -p codex-cli rate_limits`
  - `cargo test -p gemini-cli rate_limits`
  - `rg -n 'write_atomic\\(|write_timestamp\\(' crates/codex-cli/src/fs.rs crates/gemini-cli/src/fs.rs`

### Task 3.3: Migrate `gemini-cli` auth storage helpers (`auth/mod.rs`) to shared fs/json helpers and remove local duplicates
- **Location**:
  - `crates/gemini-cli/src/auth/mod.rs`
  - `crates/gemini-cli/src/auth/login.rs`
  - `crates/gemini-cli/src/auth/refresh.rs`
  - `crates/gemini-cli/src/auth/save.rs`
  - `crates/gemini-cli/src/auth/sync.rs`
  - `crates/gemini-cli/src/auth/use_secret.rs`
  - `crates/gemini-cli/src/auth/auto_refresh.rs`
- **Description**: Replace `gemini-cli` auth-local duplicated storage primitives (`write_atomic`, `write_timestamp`, newline/timestamp normalization where applicable) with `nils-common`/shared adapter usage, shrinking `auth/mod.rs` toward codex-style responsibility boundaries.
- **Dependencies**:
  - `Task 3.1`
  - `Task 3.2`
- **Complexity**: 9
- **Acceptance criteria**:
  - `gemini-cli/src/auth/mod.rs` no longer owns duplicated atomic write/timestamp primitives that already exist in shared helpers.
  - All auth command paths using these helpers preserve file permissions and timestamp behavior.
  - Existing auth tests continue to pass without output/exit-code regressions.
- **Validation**:
  - `cargo test -p gemini-cli auth`
  - `rg -n 'pub\\(crate\\) fn write_atomic|pub\\(crate\\) fn write_timestamp' crates/gemini-cli/src/auth/mod.rs`
  - `rg -n 'fn (write_atomic|write_timestamp|normalize_iso|strip_newlines)\\(' crates/gemini-cli/src/auth`

## Sprint 4: `nils-test-support` Adoption in Unit Test Modules
**Goal**: Remove bespoke test-only env/path/script helpers from source-file test modules and standardize on shared test support.
**Demo/Validation**:
- Command(s): `cargo test -p git-cli commit && cargo test -p gemini-cli auth && cargo test -p gemini-cli agent_commit`
- Verify: unit tests remain deterministic without local guard duplication.

### Task 4.1: Replace local test `EnvGuard` in `git-cli` commit tests with `nils_test_support`
- **Location**:
  - `crates/git-cli/src/commit.rs`
- **Description**: Remove the file-local test `EnvGuard` implementation and use `nils_test_support::EnvGuard` (with `GlobalStateLock`) so env mutation semantics are centralized and consistent.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 3
- **Acceptance criteria**:
  - No custom `EnvGuard` type remains in `crates/git-cli/src/commit.rs` tests.
  - Tests that mutate env vars use `GlobalStateLock` + shared guards.
  - Existing commit fixture tests continue passing.
- **Validation**:
  - `cargo test -p git-cli commit`
  - `rg -n 'struct EnvGuard' crates/git-cli/src/commit.rs`

### Task 4.2: Migrate `gemini-cli` auth source tests (`auth/login.rs`, `auth/auto_refresh.rs`) to `nils_test_support` guards/stubs
- **Location**:
  - `crates/gemini-cli/src/auth/login.rs`
  - `crates/gemini-cli/src/auth/auto_refresh.rs`
  - `crates/gemini-cli/src/auth/mod.rs`
- **Description**: Replace local test env guards, manual PATH prepend, manual executable writers, and custom temp-dir scaffolding in gemini auth source tests with `nils_test_support` equivalents (`GlobalStateLock`, `EnvGuard`, `prepend_path`, `StubBinDir`, `fs::write_executable`, `tempfile::TempDir`), and remove `auth::test_env_lock()` if no longer needed.
- **Dependencies**:
  - `Task 3.3`
- **Complexity**: 8
- **Acceptance criteria**:
  - The identified files no longer define duplicate env-guard helpers where `nils_test_support` equivalents exist.
  - PATH prepending and stub executable creation use shared helpers.
  - Auth source tests remain deterministic and pass on Unix (and non-Unix paths where covered).
- **Validation**:
  - `cargo test -p gemini-cli auth_login`
  - `cargo test -p gemini-cli auto_refresh`
  - `rg -n 'struct EnvGuard|fn env_lock\\(' crates/gemini-cli/src/auth/login.rs crates/gemini-cli/src/auth/auto_refresh.rs`

## Sprint 5: `nils-test-support` Adoption in Integration Tests and Test Harnesses
**Goal**: Converge integration-test scaffolding on shared helpers to reduce duplicated command/env/git boilerplate across crates.
**Demo/Validation**:
- Command(s): `cargo test -p agent-docs env_paths && cargo test -p gemini-cli && cargo test -p git-scope`
- Verify: test harnesses use shared helpers and remain readable/stable.

### Task 5.1: Migrate `gemini-cli` integration tests with custom env/path/temp scaffolding to `nils_test_support`
- **Location**:
  - `crates/gemini-cli/tests/paths.rs`
  - `crates/gemini-cli/tests/prompts.rs`
  - `crates/gemini-cli/tests/agent_prompt.rs`
  - `crates/gemini-cli/tests/auth_refresh.rs`
- **Description**: Replace custom `EnvVarGuard`/env lock/temp-dir/script-writing/PATH-prepend helpers with shared `nils_test_support` primitives, using existing codex test files as parity references where applicable.
- **Dependencies**:
  - `Task 1.3`
  - `Task 4.2`
- **Complexity**: 8
- **Acceptance criteria**:
  - No bespoke env guard implementations remain in `gemini-cli/tests/paths.rs` or `gemini-cli/tests/prompts.rs`.
  - `agent_prompt` and `auth_refresh` tests use shared stub/executable/path helpers instead of manual `chmod` and PATH-string composition where behavior allows.
  - Test intent and assertions remain unchanged.
- **Validation**:
  - `cargo test -p gemini-cli paths`
  - `cargo test -p gemini-cli prompts`
  - `cargo test -p gemini-cli agent_prompt`
  - `cargo test -p gemini-cli auth_refresh`

### Task 5.2: Migrate codex/gemini agent commit fallback integration tests to `nils_test_support::git` and shared fs helpers
- **Location**:
  - `crates/codex-cli/tests/agent_commit.rs`
  - `crates/gemini-cli/tests/agent_commit_fallback.rs`
- **Description**: Replace manual repo init/config/git command helpers and ad-hoc executable writes with `nils_test_support::git` and `nils_test_support::fs` helpers while preserving test coverage for fallback commit behavior.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Manual `Command::new(\"git\")` setup sequences are removed or reduced to documented exceptions.
  - Shared helpers handle repo initialization and common git commands.
  - Tests continue verifying fallback prompts/commit subject outcomes.
- **Validation**:
  - `cargo test -p codex-cli agent_commit`
  - `cargo test -p gemini-cli agent_commit_fallback`
  - `rg -n 'Command::new\\(\"git\"\\)' crates/codex-cli/tests/agent_commit.rs crates/gemini-cli/tests/agent_commit_fallback.rs`

### Task 5.3: Migrate `agent-docs` worktree path tests to shared git/fs test helpers
- **Location**:
  - `crates/agent-docs/tests/env_paths.rs`
- **Description**: Replace repeated manual git repo/worktree setup command sequences and fixture file writers with `nils_test_support::git` and `nils_test_support::fs` helpers (plus local wrappers only where worktree-specific behavior needs custom composition).
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 8
- **Acceptance criteria**:
  - Repeated `git init/config/add/commit/worktree add` setup blocks are consolidated through shared helpers and/or a local fixture builder built on them.
  - Test coverage for linked worktree detection remains unchanged.
  - File fixture writers do not duplicate parent-dir create + write behavior already provided by shared fs helpers.
- **Validation**:
  - `cargo test -p agent-docs env_paths`
  - `rg -n 'Command::new\\(\"git\"\\)' crates/agent-docs/tests/env_paths.rs`

### Task 5.4: Migrate `git-scope` manual bin resolution / allow-fail runners to shared `bin` + `cmd` helpers
- **Location**:
  - `crates/git-scope/tests/help_outside_repo.rs`
  - `crates/git-scope/tests/edge_cases.rs`
  - `crates/git-scope/tests/common.rs`
- **Description**: Replace manual binary lookup (`CARGO_BIN_EXE_*` fallback) and ad-hoc command execution plumbing in tests with `nils_test_support::bin::resolve` and `nils_test_support::cmd`, extending `tests/common.rs` only as a thin wrapper where convenient.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 5
- **Acceptance criteria**:
  - `help_outside_repo` no longer manually resolves binary paths via `CARGO_BIN_EXE_*` probing.
  - `edge_cases` allow-fail runner uses shared command helpers for captured exit/output.
  - Test assertions and coverage remain unchanged.
- **Validation**:
  - `cargo test -p git-scope help_outside_repo`
  - `cargo test -p git-scope edge_cases`
  - `rg -n 'CARGO_BIN_EXE_|std::process::Command::new\\(common::git_scope_bin\\)' crates/git-scope/tests/help_outside_repo.rs crates/git-scope/tests/edge_cases.rs`

### Task 5.5: Migrate gRPC mock integration tests to shared executable/env helpers
- **Location**:
  - `crates/api-grpc/tests/integration.rs`
  - `crates/api-test/tests/grpc_integration.rs`
  - `crates/api-testing-core/tests/suite_runner_grpc_matrix.rs`
- **Description**: Replace manual stub-script `chmod` sequences and raw env mutation with `nils_test_support::fs::write_executable` and `nils_test_support::{EnvGuard, GlobalStateLock}` (or `CmdOptions` env APIs) as appropriate.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Manual executable permission setup is removed where shared fs helper can be used.
  - Raw `unsafe` env mutation is removed in `suite_runner_grpc_matrix`.
  - gRPC mock transport tests continue passing with identical assertions.
- **Validation**:
  - `cargo test -p api-grpc integration`
  - `cargo test -p api-test grpc_integration`
  - `cargo test -p api-testing-core suite_runner_grpc_matrix`
  - `rg -n 'set_mode\\(0o755\\)|unsafe \\{ std::env::set_var|unsafe \\{ std::env::remove_var' crates/api-grpc/tests/integration.rs crates/api-test/tests/grpc_integration.rs crates/api-testing-core/tests/suite_runner_grpc_matrix.rs`

### Task 5.6: Migrate `screen-record` permission-request test stubs to shared fs helpers
- **Location**:
  - `crates/screen-record/tests/linux_request_permission.rs`
- **Description**: Replace manual ffmpeg stub writer + chmod logic with `nils_test_support` executable-writing helpers, preserving tests that intentionally use isolated `PATH` values.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 3
- **Acceptance criteria**:
  - `ffmpeg_stub_dir()` no longer manually performs write + chmod that `nils_test_support` already supports.
  - Tests still validate missing/present ffmpeg and Wayland portal scenarios.
- **Validation**:
  - `cargo test -p screen-record linux_request_permission`
  - `rg -n 'set_mode\\(0o755\\)|set_permissions\\(' crates/screen-record/tests/linux_request_permission.rs`

### Task 5.7: Sweep manual PATH-prepend test helpers into shared command/path helpers (codex/gemini/fzf)
- **Location**:
  - `crates/codex-cli/tests/agent_templates.rs`
  - `crates/codex-cli/tests/auth_json_contract.rs`
  - `crates/gemini-cli/tests/agent_templates.rs`
  - `crates/fzf-cli/tests/open_and_file.rs`
  - `crates/fzf-cli/tests/git_commands.rs`
  - `crates/fzf-cli/tests/git_commit.rs`
  - `crates/fzf-cli/tests/common.rs`
- **Description**: Replace repeated `format!(\"{}:{}\", stub, PATH)` test helpers with shared `CmdOptions::with_path_prepend` usage (or thin local wrappers around it), reducing PATH string manipulation duplication across test files.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Manual PATH-prepend string helpers are removed or reduced to one shared local wrapper per crate test harness.
  - `fzf-cli` test helpers expose a shared path-prepend path instead of repeating `path_with_stub()` in multiple files.
  - Codex/Gemini template/auth JSON tests preserve current stub resolution behavior.
- **Validation**:
  - `cargo test -p codex-cli agent_templates`
  - `cargo test -p codex-cli auth_json_contract`
  - `cargo test -p gemini-cli agent_templates`
  - `cargo test -p fzf-cli`
  - `rg -n 'format!\\(\"\\{\\}:\\{\\}\"' crates/codex-cli/tests/agent_templates.rs crates/codex-cli/tests/auth_json_contract.rs crates/gemini-cli/tests/agent_templates.rs crates/fzf-cli/tests/open_and_file.rs crates/fzf-cli/tests/git_commands.rs crates/fzf-cli/tests/git_commit.rs`

## Sprint 6: Verification, Coverage, and Closeout Audit
**Goal**: Validate all helper-adoption PRs and ensure no candidates remain untriaged.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify: required checks pass and the manifest is either fully migrated or explicitly waived per file.

### Task 6.1: Run required checks per PR and final aggregate verification
- **Location**:
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `DEVELOPMENT.md`
- **Description**: For each implementation PR, run affected crate tests plus the required repo checks before merge; for the final closeout branch, run the full required checks and workspace coverage gate.
- **Dependencies**:
  - `Task 2.1`
  - `Task 2.2`
  - `Task 2.3`
  - `Task 2.4`
  - `Task 2.5`
  - `Task 3.3`
  - `Task 4.1`
  - `Task 4.2`
  - `Task 5.7`
- **Complexity**: 5
- **Acceptance criteria**:
  - Every PR reports the targeted crate/test commands it ran.
  - Final aggregate run passes required checks and coverage gate (`>= 85%` line coverage).
  - Failures are recorded with remediation notes before merge.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

### Task 6.2: Close out the adoption manifest with `migrated` vs `keep-local` decisions
- **Location**:
  - `$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv`
  - `$AGENT_HOME/out/nils-cli-shared-helper-adoption/closeout.md`
- **Description**: Update the manifest to mark each candidate as `migrated`, `keep-local`, or `deferred`, and document the rationale for any remaining local implementations.
- **Dependencies**:
  - `Task 6.1`
- **Complexity**: 3
- **Acceptance criteria**:
  - No manifest rows remain unclassified.
  - Every `keep-local` row includes a parity/contract reason.
  - Closeout report summarizes shared helper coverage gains and deferred work.
- **Validation**:
  - `awk -F '\t' 'NR>1 && $4==\"\" {print $0}' "$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv" | wc -l | rg '^0$'`
  - `rg -n '\\t(keep-local|deferred)\\t' "$AGENT_HOME/out/nils-cli-shared-helper-adoption/manifest.tsv"`
  - `test -s "$AGENT_HOME/out/nils-cli-shared-helper-adoption/closeout.md"`

## Parallelization Notes
- Parallel after `Task 1.3`: `Task 2.1`, `Task 2.2`, `Task 2.3`, `Task 2.4`, `Task 2.5`, `Task 4.1`, `Task 5.2`, `Task 5.3`, `Task 5.4`, `Task 5.5`, `Task 5.6`, `Task 5.7`
- Sequential cluster (`nils-common` fs extraction): `Task 3.1` -> `Task 3.2` -> `Task 3.3`
- Sequential cluster (`gemini auth source tests`): `Task 3.3` -> `Task 4.2` -> `Task 5.1`
- File-overlap caution:
  - `crates/gemini-cli/src/agent/commit.rs` is owned by `Task 2.2`
  - `crates/gemini-cli/src/auth/mod.rs` is owned by `Task 3.3`
  - `crates/gemini-cli/src/auth/login.rs` and `crates/gemini-cli/src/auth/auto_refresh.rs` are owned by `Task 4.2`

## Testing Strategy
- Unit:
  - Add/update characterization tests before changing shared-helper plumbing in runtime code (`NO_COLOR`, auth path resolution, atomic-write semantics, git subprocess error handling).
  - Keep crate-local tests that verify user-facing messages and exit codes even when logic moves to shared helpers.
- Integration:
  - Prefer targeted crate tests per PR slice (the commands listed in each task) to keep feedback loops short.
  - Re-run full crate suites for high-risk shared-helper extraction tasks (`Task 3.1`–`Task 3.3`).
- E2E/manual:
  - For auth workflows (codex/gemini save/remove/refresh), use existing integration tests as contract checks; no manual-only validation should be required for merge.
- Workspace gates:
  - Final closeout must pass the required checks script and coverage gate from `DEVELOPMENT.md`.

## Risks & gotchas
- `NO_COLOR` semantics: `memo-cli` currently treats empty `NO_COLOR` differently than `nils_common::env::no_color_enabled()`. Characterize before migration.
- `codex-cli auth save/remove` may intentionally require env-only secret-dir configuration; blindly switching to provider-runtime defaults could change user-visible behavior and error messaging.
- Shared `git` wrapper adoption can accidentally change stderr/stdout capture or error formatting if callers rely on current raw `Command::new` behavior.
- Atomic-write helper extraction is high risk because permission bits, rename behavior, and error context are contract-relevant in auth/rate-limit paths.
- Test flakiness risk increases if env mutation migrations omit `GlobalStateLock` around `EnvGuard` usage.

## Rollback plan
- Land changes in small PR slices so rollback can be selective by task/cluster.
- For any regression in runtime helper extraction (`Task 2.x` / `Task 3.x`), revert the affected PR first, then keep downstream test-helper-only PRs if they are behavior-neutral.
- For shared helper API regressions (`Task 3.1`), revert the shared helper PR and any direct adopters (`Task 3.2`, `Task 3.3`) together if needed.
- Keep characterization tests in place when rolling back so the reattempt has a stable regression boundary.
- If a migration is deemed too risky, mark the manifest row `keep-local` or `deferred` in `Task 6.2` with a parity rationale rather than forcing a partial refactor.
