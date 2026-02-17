# Plan: Raise workspace line coverage to 70%

## Overview
This plan increases Rust workspace **total line coverage** from **56.89%** (9188/16150 lines hit; from `target/coverage/lcov.info`) to **at least 70.00%** using the repo’s existing `cargo llvm-cov nextest` workflow. The work focuses on adding deterministic unit/integration tests in the largest low-coverage crates (`api-testing-core`, `api-rest`, `image-processing`, `api-gql`) while avoiding behavioral changes. A buffer sprint targets `fzf-cli` only if the main sprints do not reach the 70% threshold.

## Scope
- In scope:
  - Add tests (unit + integration) that execute currently-uncovered code paths.
  - Add small refactors strictly for testability (e.g., extract pure helpers, dependency injection for I/O boundaries) when needed.
  - Add/extend fixtures under existing test directories to keep tests deterministic.
- Out of scope:
  - Feature work or CLI behavior changes unrelated to testability.
  - Performance tuning or large-scale refactors.
  - Adding new external runtime dependencies (network services, system binaries) as CI requirements.

## Assumptions (if any)
1. Coverage is generated via:
   - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
   - And summarized via: `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
2. Tests must be hermetic: use temp dirs, local loopback servers, and stub binaries (as already done in existing tests) rather than relying on developer machine state.
3. Current largest gaps by uncovered lines (approx; subject to change) are: `api-testing-core` (~2521), `fzf-cli` (~1350), `image-processing` (~847), `api-rest` (~632), `api-gql` (~456).
4. Reaching 70.00% from 56.89% requires covering roughly ~2100 additional lines (exact delta varies with code changes).

## Acceptance criteria
- `scripts/ci/coverage-summary.sh target/coverage/lcov.info` reports **Total line coverage >= 70.00%**.
- Required repo checks pass:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- New tests are deterministic (no dependence on external network/services, and no reliance on developer machine binaries beyond explicitly stubbed tools).

## Validation
- `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
- `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Sprint 1: Improve CLI frontend coverage (api-rest + api-gql)
**Goal**: Cover config/env/token resolution and command routing branches in `api-rest` and `api-gql` without changing CLI output.
**Demo/Validation**:
- Command(s):
  - `cargo test -p api-rest -p api-gql`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Tests pass locally and in CI.
  - Overall workspace coverage increases vs baseline.

**Parallel lanes**:
- Lane A: Task 1.1 + Task 1.2 + Task 1.3 (all in `api-rest`, implemented as separate new test files to minimize merge conflicts)
- Lane B: Task 1.4 (`api-gql`)

### Task 1.1: api-rest endpoint resolution tests (env/url/defaults)
- **Location**:
  - `crates/api-rest/tests/endpoint_resolution.rs`
- **Description**: Add focused integration tests that exercise `--env`, `--url`, env-as-URL (`--env https://...`), `REST_URL`, `REST_ENV_DEFAULT`, missing `endpoints.env`, and “unknown env (available: ...)” error formatting (including `.local.env` merging).
- **Dependencies**: none
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests cover precedence order and error listing behavior.
  - Assertions avoid brittle full-output matches; assert on stable substrings and exit codes.
- **Validation**:
  - `cargo test -p api-rest --test endpoint_resolution`

### Task 1.2: api-rest auth/token resolution tests (profile selection + diagnostics)
- **Location**:
  - `crates/api-rest/tests/auth_resolution.rs`
- **Description**: Add integration tests for token profile selection precedence (CLI `--token`, `REST_TOKEN_NAME`, tokens env files), unknown token profile listing, and “no tokens file present” fallbacks.
- **Dependencies**: none
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests cover: `--token` wins, env var selection, file-based selection, and unknown profile errors show available suffixes.
  - No tests require real secrets; tokens are dummy strings.
- **Validation**:
  - `cargo test -p api-rest --test auth_resolution`

### Task 1.3: api-rest report-from-cmd stdin/positional parsing coverage
- **Location**:
  - `crates/api-rest/tests/report_from_cmd.rs`
- **Description**: Extend coverage for `report-from-cmd` parsing: snippet from stdin vs positional argument, `--response -` behavior, and `--dry-run` output formatting (case name derivation + command generation).
- **Dependencies**: none
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests cover both stdin and positional snippet modes and ensure exit codes match expectations.
  - `--dry-run` output includes a syntactically valid `api-rest report ...` command.
- **Validation**:
  - `cargo test -p api-rest --test report_from_cmd`

### Task 1.4: api-gql env/jwt/list-envs/schema routing tests
- **Location**:
  - `crates/api-gql/tests/env_and_auth_resolution.rs`
  - `crates/api-gql/tests/schema_command.rs`
- **Description**: Add integration tests to cover `--list-envs`, endpoint selection (`GQL_URL_*`), JWT selection (`--jwt` + env/file precedence), and `schema` command behavior when schema files are present/missing.
- **Dependencies**: none
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests cover: listing envs, unknown env listing, and schema command success/error paths.
  - Schema tests do not require network; use temp dirs and fixture files.
- **Validation**:
  - `cargo test -p api-gql --test env_and_auth_resolution`
  - `cargo test -p api-gql --test schema_command`

## Sprint 2: Improve api-testing-core suite coverage (resolve/auth/cleanup/runner)
**Goal**: Execute the dominant uncovered code in `api-testing-core` suite modules, prioritizing deterministic logic and early-error branches before adding networked success-path tests.
**Demo/Validation**:
- Command(s):
  - `cargo test -p api-testing-core`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - `api-testing-core` coverage increases meaningfully, especially in:
    - `crates/api-testing-core/src/suite/cleanup.rs`
    - `crates/api-testing-core/src/suite/auth.rs`
    - `crates/api-testing-core/src/suite/runner.rs`
    - `crates/api-testing-core/src/suite/resolve.rs`

**Parallel lanes**:
- Lane A: Task 2.1
- Lane B: Task 2.2
- Lane C: Task 2.3
- Lane D: Task 2.5
- Lane E: Task 2.4 (starts after Task 2.1 lands)

### Task 2.1: suite/resolve.rs tests for env URL discovery and suite selection
- **Location**:
  - `crates/api-testing-core/src/suite/resolve.rs`
- **Description**: Expand unit tests to cover `find_repo_root` failure/success, `resolve_path_from_repo_root` absolute/relative behavior, `resolve_rest_base_url_for_env` / `resolve_gql_url_for_env` success and “unknown env (available: ...)” errors (including `.local.env`), and `write_file` parent-dir creation/errors.
- **Dependencies**: none
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests cover both success paths and key error messages for missing endpoints files and unknown env keys.
  - Tests use temp dirs and do not depend on real git repos (create minimal `.git/` directory only).
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.2: suite/auth.rs tests for provider selection, init, and caching semantics
- **Location**:
  - `crates/api-testing-core/src/suite/auth.rs`
- **Description**: Add unit tests for:
  - `canonical_provider` (implicit provider selection, `gql` alias, invalid combinations)
  - `init_from_suite` (missing secret env with `required=false` disables auth; invalid JSON errors)
  - `ensure_token` caching and error memoization across repeated calls
  - `resolve_auth_rest_base_url` / `resolve_auth_gql_url` precedence (override URL, defaults, env-based lookup)
- **Dependencies**: none
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests cover caching behavior (token is reused; errors are memoized) and provider validation errors.
  - Tests do not perform network I/O.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.3: suite/cleanup.rs tests for vars/template handling + early failures
- **Location**:
  - `crates/api-testing-core/src/suite/cleanup.rs`
- **Description**: Add unit tests that cover:
  - `cleanup_step_type` mapping (`gql` -> `graphql`)
  - `parse_vars_map` validation errors and null handling
  - URL resolution precedence for REST and GraphQL cleanup steps
  - Early-failure branches for invalid step inputs (missing `pathTemplate`, invalid path, missing op file, invalid `varsTemplate`, invalid `varsJq`)
  - `run_case_cleanup` error paths for missing/invalid response file and unknown step type
- **Dependencies**: none
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover at least one representative branch for each major “return Ok(false)” failure path without requiring network calls.
  - Tests assert that `main_stderr_file` log output contains the expected diagnostic prefix (stable substrings).
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.4: suite/runner.rs tests for selection/filtering and safe defaults
- **Location**:
  - `crates/api-testing-core/src/suite/runner.rs`
  - `crates/api-testing-core/src/suite/schema.rs` (only if required to add minimal fixtures)
- **Description**: Add tests that build minimal suite schemas in temp dirs and exercise:
  - Case selection/filtering behavior (`SuiteFilter` and suite defaults)
  - “no history” and “writes disabled” propagation to cleanup paths
  - Runner behavior for empty suites and invalid schemas (error formatting)
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover at least one full runner invocation that executes without network I/O (use cases that do not require HTTP execution).
  - Error messages remain stable (assert on key substrings).
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.5: Cover small 0%-coverage modules in api-testing-core
- **Location**:
  - `crates/api-testing-core/src/graphql/schema_file.rs`
  - `crates/api-testing-core/src/rest/report.rs`
- **Description**: Add unit tests that exercise the public APIs of these modules (file parsing/loading, basic report rendering) so they no longer remain at 0% coverage.
- **Dependencies**: none
- **Complexity**: 3
- **Acceptance criteria**:
  - Each module has at least one passing test that executes its main logic path.
- **Validation**:
  - `cargo test -p api-testing-core`

## Sprint 3: Improve image-processing coverage (processing + report)
**Goal**: Cover the main decision logic in `image-processing` (`processing.rs`) using dry-run flows and pure helper tests while keeping ImageMagick dependency stubbed.
**Demo/Validation**:
- Command(s):
  - `cargo test -p image-processing`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - `crates/image-processing/src/processing.rs` coverage increases substantially vs baseline.
  - No tests require real ImageMagick; all are satisfied by stub scripts.

**Parallel lanes**:
- Lane A: Task 3.1
- Lane B: Task 3.2
- Lane C: Task 3.3

### Task 3.1: Unit tests for compute_resize_box and format helpers
- **Location**:
  - `crates/image-processing/src/processing.rs`
- **Description**: Add unit tests that exhaustively cover `compute_resize_box` parameter combinations and validation errors, including:
  - `--scale` mutual exclusion rules and edge values
  - width-only, height-only, and box resize with `--fit` (`contain|cover|stretch`)
  - `--aspect` behavior with width/height and aspect mismatch detection
  - Ensure returned `(tw, th)` are clamped to at least 1
- **Dependencies**: none
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests cover both success paths and all major error messages from `compute_resize_box`.
- **Validation**:
  - `cargo test -p image-processing`

### Task 3.2: Integration tests for dry-run command construction paths
- **Location**:
  - `crates/image-processing/tests/dry_run_paths.rs`
  - `crates/image-processing/tests/common.rs`
- **Description**: Add integration tests that run the binary with stubbed `identify`/`convert`/`magick` and `--dry-run` to cover command construction branches for:
  - `convert` (alpha + background rules, quality bounds, strip metadata)
  - `resize` (fit modes, no-pre-upscale, auto-orient)
  - `rotate`, `crop`, `pad`, `optimize` (at least one “happy-path dry-run” each)
  - Output mode validation for `--out`, `--out-dir`, `--in-place` and overwrite/collision checks
- **Dependencies**: none
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests assert key command fragments appear in output/JSON, covering option-dependent branches.
  - Tests remain deterministic (no wall-clock stamps in assertions).
- **Validation**:
  - `cargo test -p image-processing --test dry_run_paths`

### Task 3.3: Add tests for report rendering
- **Location**:
  - `crates/image-processing/tests/report_rendering.rs`
- **Description**: Add unit-style integration tests for `render_report_md` to cover:
  - Dry-run true/false formatting
  - Items with and without output paths, size deltas, and error messages
  - Command list rendering and markdown structure
- **Dependencies**: none
- **Complexity**: 3
- **Acceptance criteria**:
  - `crates/image-processing/src/report.rs` is exercised and no longer 0% covered.
- **Validation**:
  - `cargo test -p image-processing --test report_rendering`

## Sprint 4: Buffer - Target fzf-cli low-coverage pure parsing modules
**Goal**: If workspace coverage is still below 70.00% after Sprint 1–3, add tests for `fzf-cli` modules that do not require `fzf` or real git interaction.
**Demo/Validation**:
- Command(s):
  - `cargo test -p fzf-cli`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Workspace total line coverage reaches or exceeds 70.00%.

**Parallel lanes**:
- Lane A: Task 4.1
- Lane B: Task 4.2

### Task 4.1: defs/index.rs parsing and file discovery tests
- **Location**:
  - `crates/fzf-cli/src/defs/index.rs`
- **Description**: Add unit tests using a temp “zsh root” directory (set `ZDOTDIR`) to cover:
  - First-party file discovery rules (include `.zshrc`/`.zprofile`, scan `scripts|bootstrap|tools`, ignore `plugins/`)
  - Alias parsing (including `alias -g`)
  - Function parsing (`function name` and `name() {` forms) and brace depth handling
  - Quote stripping for alias values
- **Dependencies**: none
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests cover both parsing success and “ignored line” cases.
  - No tests depend on the developer machine’s real Zsh config contents.
- **Validation**:
  - `cargo test -p fzf-cli defs::index`

### Task 4.2: defs/commands.rs doc rendering helpers tests
- **Location**:
  - `crates/fzf-cli/src/defs/commands.rs`
- **Description**: Add unit tests for `docblock_with_separators`, `build_alias_body`, and `build_function_body` covering indentation preservation and separator sizing.
- **Dependencies**: none
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests cover doc/no-doc cases and confirm output contains the `(from ...)` footer.
- **Validation**:
  - `cargo test -p fzf-cli defs::commands`

## Testing Strategy
- Unit:
  - Prefer unit tests inside modules for pure helpers and precedence rules (URLs, env parsing, template substitution).
- Integration:
  - Use existing “spawn binary” patterns in `crates/api-rest/tests`, `crates/api-gql/tests`, and `crates/image-processing/tests`.
  - For HTTP-dependent coverage, prefer local loopback servers started inside tests (no external network).
- E2E/manual:
  - Not required for coverage targets, but a quick smoke run of the CLIs is recommended after large test additions.

## Risks & gotchas
- Coverage totals can fluctuate as code/test structure changes; measure after each sprint and re-prioritize using the “largest uncovered lines” list.
- Some interactive `fzf-cli` code paths are intentionally hard to test without dependency injection; keep Sprint 4 focused on pure parsing/formatting modules unless refactoring is explicitly approved.
- Avoid brittle string snapshots of full CLI output; assert on stable substrings and structured JSON where available.
- Ensure tests do not leak environment variables across runs; use scoped env setters or restore env vars in `drop` guards.

## Rollback plan
- If tests introduce flakiness or slowdowns, revert the last sprint’s test additions first and reintroduce them with tighter determinism (fixed ports via `TcpListener::bind("127.0.0.1:0")`, stub binaries, and temp dirs).
- If a small refactor for testability causes risk, revert the refactor and replace with integration tests that exercise the same behavior through the CLI boundary.
