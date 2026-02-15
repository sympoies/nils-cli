# Plan: Evaluate api-rest refactor needs (maintainability + coverage)

## Overview
This plan assesses whether `crates/api-rest` needs refactoring by collecting maintainability signals,
comparing parity requirements to existing tests, and measuring crate-level coverage. The output is a
clear go/no-go decision plus a scoped refactor+test plan if gaps are material. The default is to
preserve behavior and only refactor when the evidence shows meaningful maintainability or coverage
risk.

## Scope
- In scope: `crates/api-rest` code structure, CLI command flow, helper utilities, and tests under
  `crates/api-rest/tests/` plus unit tests in `crates/api-rest/src/main.rs`.
- Out of scope: behavior changes, new CLI flags, or broad refactors in `api-testing-core` unless
  explicitly required to de-duplicate logic already shared with `api-gql`.

## Assumptions (if any)
1. Behavioral parity (output, exit codes, artifacts) is non-negotiable.
2. Coverage is evaluated with `cargo llvm-cov nextest` using the workspace’s CI tooling.
3. If `docs/plans/api-testing-core-refactor-plan.md` is actively in progress, any shared refactor
   steps will be coordinated to avoid duplication or API churn.

## Sprint 1: Baselines and evidence gathering
**Goal**: Collect maintainability and coverage signals to make a refactor decision grounded in data.
**Demo/Validation**:
- Command(s): `cargo llvm-cov nextest --profile ci -p api-rest --lcov --output-path target/coverage/api-rest.lcov.info`
- Verify: Baseline coverage and a gap list are recorded in the assessment log.

### Task 1.1: Maintainability inventory and duplication scan
- **Location**:
  - `crates/api-rest/src/main.rs`
  - `crates/api-gql/src/main.rs`
  - `docs/plans/api-rest-refactor-evaluation-plan.md`
- **Description**: Review `api-rest` for large responsibility clusters (command handling, I/O,
  formatting, env parsing), and compare with `api-gql` to identify duplicated logic that should live
  in shared modules (prefer `api-testing-core`). Summarize hotspots and potential extraction targets
  in an “Assessment log” section appended to this plan.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Hotspots are listed with concrete function/section references.
  - At least one candidate for shared extraction vs local module split is identified.
- **Validation**:
  - `rg -n "fn cmd_|resolve_" crates/api-rest/src/main.rs crates/api-gql/src/main.rs`

### Task 1.2: Capture api-rest coverage baseline and hotspots
- **Location**:
  - `target/coverage/api-rest.lcov.info`
  - `docs/plans/api-rest-refactor-evaluation-plan.md`
- **Description**: Run crate-scoped coverage, extract overall line coverage, and list the top
  uncovered functions/blocks. Record the baseline % and the top 5 coverage hotspots in the
  assessment log.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Baseline api-rest line coverage is recorded with numeric % and hit/miss counts.
  - Top 5 uncovered areas are listed with file/line references.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci -p api-rest --lcov --output-path target/coverage/api-rest.lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/api-rest.lcov.info`

### Task 1.3: Parity spec → tests gap map
- **Location**:
  - `crates/api-rest/README.md`
  - `crates/api-rest/tests/integration.rs`
  - `crates/api-rest/tests/auth_resolution.rs`
  - `crates/api-rest/tests/endpoint_resolution.rs`
  - `crates/api-rest/tests/report_from_cmd.rs`
  - `crates/api-rest/tests/cli_smoke.rs`
  - `docs/plans/api-rest-refactor-evaluation-plan.md`
- **Description**: Map parity-critical behaviors (history rotation, report flags, env precedence,
  error paths, stdin/response modes) to existing tests. Record gaps that are not covered by unit or
  integration tests, with suggested test types (unit vs integration).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Each major README section (call/history/report) is marked as covered or missing.
  - Missing behaviors are listed with a suggested test location.
- **Validation**:
  - `rg -n "history|report|expect|cleanup|jwt|token" crates/api-rest/tests crates/api-rest/src/main.rs`

## Sprint 2: Refactor decision and design (conditional)
**Goal**: Decide whether refactor is warranted and define the smallest safe scope if needed.
**Demo/Validation**:
- Command(s): none (decision gate)
- Verify: A decision and scoped design are recorded.

### Task 2.1: Decision gate based on evidence
- **Location**:
  - `docs/plans/api-rest-refactor-evaluation-plan.md`
- **Description**: Using Sprint 1 evidence, decide one of: (A) no refactor needed, (B) local
  module split only, or (C) shared extraction with `api-testing-core`. Record the decision, scope,
  and rationale in the assessment log.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Decision includes objective triggers (example: coverage below 80%, main.rs above 1500 LOC, or
    duplicated endpoint/auth logic with api-gql that cannot be trivially shared).
  - If no refactor, list minimal test-only improvements.
- **Validation**:
  - n/a (documented decision)

### Task 2.2: Refactor design sketch (if decision B/C)
- **Location**:
  - `crates/api-rest/src/main.rs`
  - `crates/api-rest/src/cli.rs`
  - `crates/api-rest/src/commands/call.rs`
  - `crates/api-rest/src/commands/history.rs`
  - `crates/api-rest/src/commands/report.rs`
  - `crates/api-rest/src/commands/report_from_cmd.rs`
  - `crates/api-rest/src/util.rs`
  - `docs/plans/api-rest-refactor-evaluation-plan.md`
- **Description**: Draft a minimal module layout and migration steps. Example: move CLI structs to
  `cli.rs`, command handlers to `commands/{call,history,report}.rs`, and helpers to `util.rs`. For
  shared extraction, list the exact APIs to move into `api-testing-core` (prefer reusing or aligning
  with the existing core refactor plan).
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Proposed module layout lists concrete files and responsibilities.
  - Shared extraction candidates are aligned with `api-testing-core` APIs and tests.
- **Validation**:
  - `rg -n "cmd_call|cmd_report|cmd_history" crates/api-rest/src/main.rs`

### Task 2.3: Test gap plan (if decision B/C)
- **Location**:
  - `crates/api-rest/tests/integration.rs`
  - `crates/api-rest/tests/history.rs`
  - `crates/api-rest/tests/report.rs`
  - `crates/api-rest/tests/report_from_cmd.rs`
  - `crates/api-rest/src/main.rs`
  - `docs/plans/api-rest-refactor-evaluation-plan.md`
- **Description**: Translate the gap map into a prioritized test list (unit vs integration),
  including new fixtures for history rotation, report redaction toggles, stdin response handling,
  and error paths. Specify which tests are added to `api-rest` vs those better placed in
  `api-testing-core`.
- **Dependencies**:
  - Task 2.1
  - Task 1.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Each gap has a concrete test location and outline of assertions.
  - Coverage impact is estimated (which functions/blocks will be exercised).
- **Validation**:
  - n/a (plan-only)

## Sprint 3: Implementation + coverage (conditional)
**Goal**: Apply the scoped refactor and close coverage gaps without changing behavior.
**Demo/Validation**:
- Command(s): `cargo test -p api-rest`, `cargo llvm-cov nextest --profile ci -p api-rest --lcov --output-path target/coverage/api-rest.lcov.info`
- Verify: All api-rest tests pass and coverage increases vs baseline.

### Task 3.1: Refactor module layout with zero behavior change
- **Location**:
  - `crates/api-rest/src/main.rs`
  - `crates/api-rest/src/cli.rs`
  - `crates/api-rest/src/commands/call.rs`
  - `crates/api-rest/src/commands/history.rs`
  - `crates/api-rest/src/commands/report.rs`
  - `crates/api-rest/src/commands/report_from_cmd.rs`
  - `crates/api-rest/src/util.rs`
- **Description**: Split the monolithic `main.rs` into smaller modules per the design sketch while
  keeping CLI behavior identical. Ensure `main.rs` only wires CLI parsing to command handlers.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - No observable CLI output changes for existing tests.
  - Module boundaries are clear and reduce function size/duplication.
- **Validation**:
  - `cargo test -p api-rest`

### Task 3.2: Add targeted tests to close parity gaps
- **Location**:
  - `crates/api-rest/tests/integration.rs`
  - `crates/api-rest/tests/history.rs`
  - `crates/api-rest/tests/report.rs`
  - `crates/api-rest/tests/report_from_cmd.rs`
  - `crates/api-rest/tests/endpoint_resolution.rs`
  - `crates/api-rest/tests/auth_resolution.rs`
  - `crates/api-rest/src/main.rs`
- **Description**: Implement the test additions from Task 2.3, favoring hermetic tests using
  `nils-test-support` and loopback HTTP servers. Cover history rotation, report redaction flags,
  stdin/response modes, and error messages for invalid inputs.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Missing parity behaviors from Sprint 1 are covered by new tests.
  - Tests are deterministic and do not rely on external network.
- **Validation**:
  - `cargo test -p api-rest --tests`

### Task 3.3: Coverage checkpoint and repo gates
- **Location**:
  - `target/coverage/api-rest.lcov.info`
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- **Description**: Re-run coverage and repo-required checks to confirm no regressions.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 4
- **Acceptance criteria**:
  - api-rest line coverage increases vs baseline and CI coverage gate remains green.
  - Required workspace checks pass.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci -p api-rest --lcov --output-path target/coverage/api-rest.lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/api-rest.lcov.info`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: Expand helper/function tests in `crates/api-rest/src/main.rs` (or new modules) for env
  parsing, command building, and report snippet logic.
- Integration: Use `nils-test-support` loopback HTTP server for call/report paths and history file
  assertions under temp `setup/rest` directories.
- E2E/manual: Run the repo’s required checks (`nils-cli-checks`) once changes land.

## Risks & gotchas
- Shared refactor overlap with `api-testing-core` plan can cause duplicated work or API churn.
- Coverage improvements may require test fixtures that slightly increase the coverage denominator.
- History/report formatting is parity-sensitive; small refactors can unintentionally alter output.

## Rollback plan
- If refactor introduces output diffs or brittle tests, revert to the pre-refactor module layout and
  keep only non-invasive tests that preserve parity.

## Assessment log
- Sprint 1.1: Maintainability inventory (hotspots + duplication)
  - Hotspots (responsibility clusters):
    - `crates/api-rest/src/main.rs:461` (`resolve_endpoint_for_call`/`resolve_auth_for_call`/`validate_bearer_token_if_jwt`/`cmd_call_internal`): endpoint + auth + JWT validation + execution + history wiring in one block.
    - `crates/api-rest/src/main.rs:900` (`append_history_best_effort`/`cmd_history`): history record formatting, file I/O, and CLI output in one block.
    - `crates/api-rest/src/main.rs:1046` (`cmd_report`/`cmd_report_from_cmd`): report generation, redaction, input modes, file output, and error handling are tightly coupled.
    - `crates/api-gql/src/main.rs:550` (`resolve_endpoint_for_call`/`cmd_call_internal`/`append_history_best_effort`): endpoint + execution + history in one block.
    - `crates/api-gql/src/main.rs:1123` (`cmd_report`/`cmd_report_from_cmd`): report generation with validation logic (e.g., `response_has_meaningful_data_records`).
  - Duplication / extraction candidates:
    - `resolve_endpoint_for_call` (REST/GQL are structurally similar, mainly defaults and labels differ).
    - `cmd_history` + history record formatting (REST/GQL share setup-dir discovery + read/print flow).
    - Timestamp helpers (`history_timestamp_now`, `report_stamp_now`, `report_date_now`) with near-identical formatting.
  - Shared vs local split candidates:
    - Shared (defer to `api-testing-core` plan): endpoint resolution + history record formatting + timestamp helpers.
    - Local (low-risk): move `cmd_call_internal`, `cmd_report`, `cmd_history` into `commands/{call,report,history}.rs`.
- Sprint 1.2: Coverage baseline
  - `api-rest` line coverage: **76.92% (1000/1300 lines hit)**; 300 lines missed.
  - Top 5 uncovered segments (by consecutive missed lines, all in `crates/api-rest/src/main.rs`):
    - `main.rs:1148-1169` (22 lines)
    - `main.rs:814-823` (10 lines)
    - `main.rs:590-598` (9 lines)
    - `main.rs:645-653` (9 lines)
    - `main.rs:1104-1112` (9 lines)
- Sprint 1.3: Parity spec → tests gap map
  - Call:
    - Covered: endpoint/auth resolution (`endpoint_resolution.rs`, `auth_resolution.rs`), basic `expect` paths + cleanup (`integration.rs`), CLI smoke (`cli_smoke.rs`).
    - Missing: request schema validation, query encoding rules, default/header behavior, multipart precedence, cleanup failure paths, failure-body echo, JWT strict/expiry warnings, setup-dir discovery (unit + integration gaps).
  - History:
    - Covered: none beyond CLI help in `cli_smoke.rs`.
    - Missing: write conditions (`--no-history`, env toggles), path resolution, record formatting (redaction + command snippet), URL logging toggle, rotation, `api-rest history` flags, empty-file exit behavior (integration gaps).
  - Report:
    - Covered: `report-from-cmd` generation and basic stdin/flag conflicts (`report_from_cmd.rs`, `cli_smoke.rs`).
    - Missing: `api-rest report` core flows, output path defaults, markdown structure, redaction depth, `--no-command` flags, expect/assertion outcomes (unit + integration gaps).
- Sprint 2.1: Decision gate
  - Decision: **B — local module split only**, plus targeted test additions (no shared extraction yet).
  - Triggers:
    - `crates/api-rest/src/main.rs` is **1870 lines** (threshold >1500).
    - Coverage baseline **76.92%** (<80%).
    - Duplicated endpoint/history/report logic vs `api-gql` observed.
  - Rationale:
    - Local module split reduces `main.rs` responsibility without cross-crate churn.
    - Shared extraction is deferred until alignment with `docs/plans/api-testing-core-refactor-plan.md` to avoid API churn.
  - Follow-up:
    - Proceed with design sketch + test gap plan; focus on local module layout and test coverage to close parity gaps.
- Sprint 2.2: Refactor design sketch (local split)
  - `main.rs`: keep `main()`/`run()` and command dispatch; keep `print_root_help` + `argv_with_default_command`.
  - `cli.rs`: move clap structs (`Cli`, `Command`, `CallArgs`, `HistoryArgs`, `ReportArgs`, `ReportFromCmdArgs`).
  - `commands/call.rs`: `cmd_call`, `cmd_call_internal`, endpoint/auth/JWT helpers, history append wiring.
  - `commands/history.rs`: `cmd_history` and history-specific helpers.
  - `commands/report.rs`: `cmd_report`, `cmd_report_from_cmd`, report command snippet builders.
  - `util.rs`: shared helpers (`trim_non_empty`, `bool_from_env`, `parse_u64_default`, `to_env_key`, `slugify`,
    `maybe_relpath`, `shell_quote`, `list_available_suffixes`, `find_git_root`, timestamps).
  - Tests: move unit tests into corresponding modules; use `pub(crate)` visibility where needed for tests.
- Sprint 2.3: Test gap plan (prioritized for this refactor)
  - P0 History (new `crates/api-rest/tests/history.rs`):
    - `api-rest call` writes `.rest_history` with stamp/exit/setup_dir + command snippet.
    - `--no-history` and `REST_HISTORY_ENABLED=false` skip write.
    - `REST_HISTORY_LOG_URL_ENABLED=false` omits url in record.
    - `REST_HISTORY_FILE` override writes to custom path.
    - `api-rest history --tail/--last/--command-only` output format checks.
  - P0 Report (new `crates/api-rest/tests/report.rs`):
    - `--response <file>` writes report and includes key sections.
    - `--response -` reads stdin.
    - `--run` mode against loopback server yields PASS result.
    - `--no-redact` vs default redaction difference.
    - `--no-command` and `--no-command-url` change snippet content.
  - P0 Call error path (extend `crates/api-rest/tests/integration.rs`):
    - Non-JSON response body on expect failure prints stderr “Response body (non-JSON; …)”.
    - Cleanup failure returns non-zero and surfaces error.
  - Deferred (explicitly out of scope for this refactor):
    - Request schema/query/header/multipart precedence edge cases.
    - JWT strict/expiry warning coverage.
    - Setup-dir discovery error paths.
    - Report jq assertion states in `--response` mode.
  - Coverage impact: expect to exercise `cmd_history`, `append_history_best_effort`, `cmd_report` branches and
    reduce uncovered hotspots around report/history ranges.
- Sprint 3.1: Local module split implemented
  - New modules: `cli.rs`, `util.rs`, `commands/{call,history,report}.rs`, `commands/mod.rs`.
  - `main.rs` now focuses on CLI dispatch; behavior unchanged.
- Sprint 3.2: Targeted tests added (P0 scope)
  - New integration tests: `crates/api-rest/tests/history.rs`, `crates/api-rest/tests/report.rs`.
  - Added call error-path coverage in `crates/api-rest/tests/integration.rs`.
- Sprint 3.3: Coverage + required checks
  - Coverage: **83.59% (1131/1353 lines hit)** vs baseline 76.92%.
  - Required checks: `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh` passed.
- Follow-up: Deferred gaps closed
  - JWT strict/expiry validations covered (unit tests in `crates/api-rest/src/commands/call.rs`).
  - Schema/query/header/multipart edge cases covered (`crates/api-rest/tests/schema_edges.rs`).
  - Setup-dir discovery covered (`crates/api-rest/tests/setup_resolution.rs`).
  - Report jq assertions in `--response` mode covered (`crates/api-rest/tests/report.rs`).
  - Coverage: **88.01% (1241/1410 lines hit)** after follow-up tests.
  - Required checks re-run: `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`.
