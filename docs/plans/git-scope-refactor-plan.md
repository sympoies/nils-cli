# Plan: git-scope maintainability + efficiency refactor

## Overview
This plan refactors internal git-scope modules to reduce duplication and improve readability while preserving byte-for-byte output parity. It introduces shared Git command helpers, a unified change-parsing model, and centralized rendering/printing pipelines. Efficiency improvements focus on caching external tool capability checks and reducing repeated parsing, with maintainability taking priority when tradeoffs exist. Work is staged with characterization tests to keep regressions visible.

## Scope
- In scope: internal module restructuring, shared helpers for Git execution and change parsing, tree/print pipeline consolidation, tool capability caching, new unit/integration tests to lock behavior.
- Out of scope: new features or flags, changes to output text/emoji/order, switching away from shelling out to git, altering test fixtures.

## Assumptions (if any)
1. Behavioral parity (stdout/stderr + exit codes) is required across all subcommands.
2. The crate continues to rely on external tools (git, tree, file, mktemp) with the same warnings and fallbacks.
3. Tests can create temporary git repos and set env vars like NO_COLOR and GIT_SCOPE_PROGRESS.

## Parallelization
- Task 1.1 and Task 1.2 can run in parallel.
- Task 1.3 can run in parallel with Task 1.1/1.2; Task 1.4 follows Task 1.3.
- Task 2.1 and Task 2.3 can run in parallel after Task 1.4.
- Task 2.2 follows Task 2.1; Task 2.4 follows Task 2.3.
- Task 2.5 can run after Task 2.4, and Task 2.6 follows Task 2.5.
- Task 3.1 and Task 3.2 can run in parallel after Task 2.4.

## Sprint 1: Characterization + core helpers
**Goal**: Lock current behavior with tests and establish shared Git command helpers.
**Demo/Validation**:
- Command(s): `cargo test -p git-scope --test rendering`, `cargo test -p git-scope --test tool_degradation`
- Verify: characterization tests pass without changing any CLI output.

### Task 1.1: Add characterization tests for command output structure
- **Location**:
  - `crates/git-scope/tests/characterization_commands.rs`
- **Description**: Add integration tests that assert headings and ordering for tracked, staged, unstaged, all, untracked, and commit outputs so refactors cannot change output format.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Each subcommand test asserts the section headers in order (for example: Changed files, Directory tree, Printing file contents).
  - Tests run with NO_COLOR=1 to avoid ANSI variance.
- **Validation**:
  - `cargo test -p git-scope --test characterization_commands`

### Task 1.2: Add characterization tests for warnings and edge behavior
- **Location**:
  - `crates/git-scope/tests/characterization_warnings.rs`
- **Description**: Add tests that assert warning strings for invalid --parent, missing tree, and missing file fallback paths so refactors keep error text stable.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Invalid --parent emits the same warning text as current behavior.
  - Missing tree emits the same warning text as current behavior.
- **Validation**:
  - `cargo test -p git-scope --test characterization_warnings`

### Task 1.3: Centralize Git command execution helpers
- **Location**:
  - `crates/git-scope/src/git_cmd.rs`
  - `crates/git-scope/src/git.rs`
  - `crates/git-scope/src/commit.rs`
- **Description**: Introduce a shared Git command helper module that wraps command execution, configures core.quotepath=false by default, and standardizes error context; update callers in git.rs and commit.rs.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - git.rs and commit.rs no longer define their own run_git helpers.
  - Error contexts remain as informative as before (command + args).
- **Validation**:
  - `cargo test -p git-scope`

### Task 1.4: Create a shared change-line model and parser
- **Location**:
  - `crates/git-scope/src/change.rs`
  - `crates/git-scope/src/render.rs`
  - `crates/git-scope/src/commit.rs`
- **Description**: Add a ChangeEntry struct with helpers (is_rename_or_copy, display_path, file_path) and a parser for name-status lines so render and commit code share the same logic.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - render.rs and commit.rs use the shared parser/helpers.
  - Rename/copy handling is implemented once and reused.
- **Validation**:
  - `cargo test -p git-scope --test rendering`

## Sprint 2: Rendering + print pipeline consolidation
**Goal**: Reduce duplication in tree rendering and file printing, with safe efficiency wins.
**Demo/Validation**:
- Command(s): `cargo test -p git-scope --test characterization_commands`, `cargo test -p git-scope --test print_sources`
- Verify: refactors keep output sections and print labels stable.

### Task 2.1: Extract tree capability cache module
- **Location**:
  - `crates/git-scope/src/tree.rs`
- **Description**: Implement cached detection of tree availability and --fromfile support using OnceLock, returning a TreeSupport struct with flags and warnings.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - tree --version and tree --fromfile checks are performed once per process.
  - The cache exposes a single entry point for callers to query capability.
- **Validation**:
  - `cargo test -p git-scope --test rendering`

### Task 2.2: Wire tree rendering through the cached module
- **Location**:
  - `crates/git-scope/src/render.rs`
  - `crates/git-scope/src/commit.rs`
- **Description**: Replace direct tree capability checks with tree::TreeSupport usage in both worktree and commit render paths, preserving existing warning text.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Warning messages remain identical when tree is missing or unsupported.
  - Tree output remains unchanged when tree is available.
- **Validation**:
  - `cargo test -p git-scope --test rendering`
  - `cargo test -p git-scope --test tool_degradation`

### Task 2.3: Introduce a shared print helper API
- **Location**:
  - `crates/git-scope/src/print.rs`
- **Description**: Define a PrintSource enum and a function such as print::emit_file(source: PrintSource, path: &str, fallback: HeadFallback) that prints headers, content, and binary placeholders consistently.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - A single helper handles worktree, index, and HEAD fallback formatting.
  - Output labels and placeholders match current behavior for text and binary files.
- **Validation**:
  - `cargo test -p git-scope --test print_sources`

### Task 2.4: Replace call sites with the shared print helper
- **Location**:
  - `crates/git-scope/src/render.rs`
  - `crates/git-scope/src/commit.rs`
- **Description**: Update render_with_type, print_all_files, and render_commit to call the new print helper, removing duplicated code paths.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 4
- **Acceptance criteria**:
  - render.rs and commit.rs no longer duplicate print label or fallback logic.
  - Print outputs are byte-for-byte identical to pre-refactor behavior.
- **Validation**:
  - `cargo test -p git-scope --test print_sources`

### Task 2.5: Create a progress runner helper
- **Location**:
  - `crates/git-scope/src/progress.rs`
- **Description**: Add a small helper that runs a list of operations with optional progress display, keeping stdout stable when GIT_SCOPE_PROGRESS is enabled.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Progress helper can run zero or more operations without emitting stderr in non-TTY contexts.
  - Helper abstracts Progress initialization and teardown in one place.
- **Validation**:
  - `cargo test -p git-scope --test progress_opt_in`

### Task 2.6: Update render/commit to use the progress helper
- **Location**:
  - `crates/git-scope/src/render.rs`
  - `crates/git-scope/src/commit.rs`
- **Description**: Replace inlined progress loops with the shared progress helper in render_with_type, print_all_files, and render_commit.
- **Dependencies**:
  - Task 2.5
- **Complexity**: 3
- **Acceptance criteria**:
  - Progress behavior (GIT_SCOPE_PROGRESS=1) remains stdout-stable and silent on stderr in non-TTY tests.
  - Progress handling code is removed from render.rs and commit.rs call sites.
- **Validation**:
  - `cargo test -p git-scope --test progress_opt_in`

## Sprint 3: Efficiency polish + validation
**Goal**: Apply low-risk efficiency improvements and finalize validation coverage.
**Demo/Validation**:
- Command(s): `cargo test -p git-scope`, `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify: all required checks pass.

### Task 3.1: Cache file command availability and avoid repeated spawn errors
- **Location**:
  - `crates/git-scope/src/print.rs`
- **Description**: Detect file --mime availability once (via OnceLock) and route to the fallback binary check when missing, avoiding repeated error handling per file.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 3
- **Acceptance criteria**:
  - When file is missing, printing still works and uses the fallback detector.
  - No new warnings are emitted in the missing-file path.
- **Validation**:
  - `cargo test -p git-scope --test tool_degradation`

### Task 3.2: Add unit tests for change parsing and path canonicalization
- **Location**:
  - `crates/git-scope/src/change.rs`
- **Description**: Add unit tests in change.rs that cover rename/copy parsing, display_path, file_path, and canonical path handling for => patterns used in commit numstat.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests cover at least one rename (R100) and one brace-style => path.
  - Parsing outputs match existing render/commit expectations.
- **Validation**:
  - `cargo test -p git-scope change_`

### Task 3.3: Run full required checks before delivery
- **Location**:
  - `DEVELOPMENT.md`
- **Description**: Execute the repo-required lint/test suite to confirm no regressions before merging refactor work.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
  - Task 1.4
  - Task 2.1
  - Task 2.2
  - Task 2.3
  - Task 2.4
  - Task 2.5
  - Task 2.6
  - Task 3.1
  - Task 3.2
- **Complexity**: 2
- **Acceptance criteria**:
  - cargo fmt, cargo clippy, cargo test --workspace, and zsh completion tests pass.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Testing Strategy
- Unit: change parsing tests and binary detection fallback tests.
- Integration: existing git-scope tests plus added characterization tests for output invariants.
- E2E/manual: optional spot-check in a temp repo for staged -p, all -p, and commit HEAD -p with NO_COLOR=1.

## Risks & gotchas
- Subtle output formatting changes (spacing, emojis, order) could break parity; mitigate with characterization tests.
- OnceLock caching can introduce test order coupling; avoid by keeping capability detection consistent per test binary and validating with isolated test binaries where needed.
- Consolidating print logic could accidentally change HEAD fallback labels; ensure tests cover these cases.

## Rollback plan
- Revert the refactor commits as a single batch (or git revert by sprint) to restore the prior implementation.
- Keep added characterization tests to guard future refactors, even if code is rolled back.
