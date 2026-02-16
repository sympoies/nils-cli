# Plan: memo-agent inbox CLI

## Overview
This plan delivers a new Rust CLI crate for capture-first personal memo workflows backed by an
embedded SQLite database and agent-driven post-processing. The MVP keeps raw inbox entries durable
and immutable, then lets an agent write structured derivations for query and reporting. Human mode
remains the default UX, while JSON mode is explicit for machine consumption by agent scripts and
service callers. The rollout prioritizes reliable data capture, deterministic query behavior, and
contract-tested CLI output.

## Scope
- In scope: new workspace crate (`nils-memo-cli`), SQLite storage with FTS5, command set
  (`add`, `list`, `search`, `report`, `fetch`, `apply`), human output contract, JSON contract,
  tests, completions, and delivery gates.
- Out of scope: cloud sync, multi-device conflict resolution, vector embedding search, background
  daemon/server mode, and GUI/mobile clients.

## Assumptions (if any)
1. Source requirements come from `/Users/terry/Downloads/Rust Embedded Database Choices.md`,
   especially the capture-first inbox model and `fetch` plus `apply` agent loop.
2. MVP focuses on SQLite plus FTS5 first; semantic vector search is deferred until data volume and
   retrieval quality justify added complexity.
3. Raw inbox records are append-only; corrections are represented by new entries or derivations.
4. First release targets crates.io publication; crate metadata, publish order, and dry-run gates
   are required before MVP delivery can be considered complete.

## Success Criteria
1. Users can record free-form memos, list records, keyword-search records, and view weekly or
   monthly summaries through one CLI.
2. Agent scripts can pull unprocessed records with `fetch --json` and write normalized derivations
   with `apply --json`.
3. JSON mode follows `docs/specs/cli-service-json-contract-guideline-v1.md` with stable envelope
   keys and structured errors.
4. Required checks in `DEVELOPMENT.md` pass, and publishability policy is explicitly documented.

## Standards alignment (nils-cli-create-cli-crate)
- Follow `docs/runbooks/new-cli-crate-development-standard.md` for crate scaffold workflow, human
  output rules, JSON mode requirements, and publish-readiness decisions.
- Follow `docs/specs/cli-service-json-contract-guideline-v1.md` for envelope fields
  (`schema_version`, `command`, `ok`) and structured `error` payload requirements.
- Keep machine-facing contracts versioned and test-backed; keep human output readable and stable.

## Sprint 1: Product contract and storage design freeze
**Goal**: Lock the MVP command, schema, and JSON contract decisions before implementation.
**Demo/Validation**:
- Command(s): `plan-tooling validate --file docs/plans/memo-agent-inbox-cli-plan.md`
- Verify: plan remains executable with complete dependencies and no unresolved fields.

### Task 1.1: Write command contract and workflow spec
- **Location**:
  - `crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md`
  - `crates/memo-cli/README.md`
- **Description**: Define command semantics for `add`, `list`, `search`, `report`, `fetch`, and
  `apply`, including argument flags, exit code policy, stdout vs stderr boundaries, and explicit
  JSON mode behavior for service/agent usage.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Spec covers at least one end-to-end flow from quick capture to agent enrichment to reporting.
  - Spec defines `--json` and `--format json` behavior consistently across machine-facing commands.
  - README usage examples align with the command contract document.
  - Text-mode defaults and JSON opt-in behavior are explicitly documented.
- **Validation**:
  - `rg -n "add|list|search|report|fetch|apply|--json|--format json" crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md`
  - `rg -n "capture|enrichment|fetch|apply|report|stdout|stderr|exit code" crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md`
  - `rg -n "Usage|Commands|JSON" crates/memo-cli/README.md`

### Task 1.2: Define SQLite schema and lifecycle rules
- **Location**:
  - `crates/memo-cli/docs/specs/memo-cli-storage-schema-v1.md`
  - `crates/memo-cli/src/storage/sql/schema_v1.sql`
- **Description**: Define schema and lifecycle rules for `inbox_items`, `item_derivations`,
  `tags`, `item_tags`, and FTS index tables, including immutability of raw records, active
  derivation selection, and safe reprocessing semantics.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Schema doc describes key columns, constraints, and index strategy for list/search/report paths.
  - FTS5 indexing strategy is specified for raw text and selected derived fields.
  - Derivation versioning and active selection logic are documented with conflict handling rules.
  - SQL schema file and spec stay aligned on table and column names.
- **Validation**:
  - `rg -n "inbox_items|item_derivations|tags|item_tags|fts5" crates/memo-cli/docs/specs/memo-cli-storage-schema-v1.md`
  - `rg -n "create table|inbox_items|item_derivations|fts" crates/memo-cli/src/storage/sql/schema_v1.sql`

### Task 1.3: Define machine-consumable JSON contract v1
- **Location**:
  - `crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`
  - `docs/specs/cli-service-json-contract-guideline-v1.md`
- **Description**: Specify success and error envelopes for all JSON-capable commands, including
  stable error codes, required top-level keys, and representative examples for `fetch`, `apply`,
  `search`, and `report`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Contract uses required envelope keys and command identifiers for all examples.
  - Single-object responses use `result`; collections use `results`.
  - Error payload examples include `code` and `message`, with optional structured `details`.
  - Contract explicitly states that sensitive fields are never emitted.
- **Validation**:
  - `rg -n "\"schema_version\"|\"command\"|\"ok\"|\"result\"|\"results\"|\"error\"" crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`
  - `rg -n "sensitive|redact|token|secret|code|message" crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`

### Task 1.4: Record crate publishability policy and release gates
- **Location**:
  - `crates/memo-cli/docs/specs/memo-cli-release-policy.md`
  - `crates/memo-cli/Cargo.toml`
  - `release/crates-io-publish-order.txt`
- **Description**: Define and document publishable-first release policy for MVP, including required
  Cargo metadata, crates.io publish order placement, and dry-run verification gates.
- **Dependencies**:
  - Task 1.1
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Policy explicitly states first release is publishable and explains rationale.
  - `crates/memo-cli/Cargo.toml` is configured for crates.io publication.
  - `release/crates-io-publish-order.txt` contains `nils-memo-cli` in dependency-safe position.
  - Policy includes required dry-run verification command for publish readiness.
- **Validation**:
  - `rg -n "publishable-first|crates.io|release order|dry-run" crates/memo-cli/docs/specs/memo-cli-release-policy.md`
  - `rg -n "^name = \"nils-memo-cli\"|^version = |^description = |^repository = " crates/memo-cli/Cargo.toml`
  - `bash -lc '! rg -n "^publish = false" crates/memo-cli/Cargo.toml'`
  - `rg -n "nils-memo-cli" release/crates-io-publish-order.txt`

## Sprint 2: Crate scaffold and capture/query core
**Goal**: Build a usable CLI that can capture memos and query them through list/search/report.
**Demo/Validation**:
- Command(s): `cargo run -p nils-memo-cli -- --help`
- Verify: command surface is wired and core storage paths execute against a local SQLite file.

### Task 2.1: Scaffold new crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/memo-cli/Cargo.toml`
  - `crates/memo-cli/src/main.rs`
  - `crates/memo-cli/src/lib.rs`
- **Description**: Create the new crate, wire workspace membership, configure package metadata, and
  establish baseline module layout for CLI, storage, rendering, and JSON output layers.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Workspace includes the new crate and `cargo check -p nils-memo-cli` succeeds.
  - Package metadata matches repository conventions for the selected release policy.
  - Baseline crate layout supports isolated testing of parser, storage, and render paths.
  - Running binary help output succeeds with no panic.
- **Validation**:
  - `cargo check -p nils-memo-cli`
  - `cargo run -p nils-memo-cli -- --help`

### Task 2.2: Implement CLI parser and command dispatch
- **Location**:
  - `crates/memo-cli/src/cli.rs`
  - `crates/memo-cli/src/main.rs`
  - `crates/memo-cli/src/commands/mod.rs`
- **Description**: Implement clap command parsing and dispatch for all MVP subcommands, including
  shared flags for database path, output mode, and deterministic sorting controls.
- **Dependencies**:
  - Task 1.1
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - All six MVP subcommands are available in `--help` output with stable descriptions.
  - Parser rejects invalid option combinations with documented usage exit code behavior.
  - Shared output mode flag handling is centralized and reusable by command handlers.
  - Dispatch layer is covered by parser-focused tests.
- **Validation**:
  - `cargo run -p nils-memo-cli -- --help`
  - `cargo run -p nils-memo-cli -- add --help`
  - `cargo run -p nils-memo-cli -- fetch --help`
  - `cargo test -p nils-memo-cli cli::tests`

### Task 2.3: Implement SQLite bootstrap and migration path
- **Location**:
  - `crates/memo-cli/src/storage/mod.rs`
  - `crates/memo-cli/src/storage/sql/schema_v1.sql`
  - `crates/memo-cli/src/storage/migrate.rs`
- **Description**: Implement database initialization, migration bookkeeping, WAL configuration,
  foreign-key enforcement, and transaction wrappers needed by capture, search, and agent flows.
- **Dependencies**:
  - Task 1.2
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - First run initializes schema and records migration state without manual setup.
  - Repeated startup is idempotent and does not rewrite existing data.
  - Storage layer exposes transactional APIs for write and read workloads.
  - Migration tests cover fresh database and already-initialized database cases.
- **Validation**:
  - `cargo test -p nils-memo-cli storage::tests::init_db`
  - `cargo test -p nils-memo-cli storage::tests::migration_idempotent`

### Task 2.4: Implement `add` and `list` on raw inbox data
- **Location**:
  - `crates/memo-cli/src/commands/add.rs`
  - `crates/memo-cli/src/commands/list.rs`
  - `crates/memo-cli/src/storage/repository.rs`
- **Description**: Implement durable capture and deterministic listing of raw memo entries with
  filtering, pagination limits, and ordering that remain stable across repeated executions.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `add` persists raw text and returns record identity in text and JSON modes.
  - `list` returns deterministic ordering by created time and identifier tie-breaker.
  - Raw text is never mutated by list or derived-data operations.
  - Tests cover empty list, single record, and multi-record ordering behavior.
- **Validation**:
  - `cargo test -p nils-memo-cli add_and_list`
  - `cargo test -p nils-memo-cli add_and_list_json`
  - `cargo run -p nils-memo-cli -- add "buy 1tb ssd for mom"`
  - `cargo run -p nils-memo-cli -- add --json "book two parenting books"`
  - `cargo run -p nils-memo-cli -- list --limit 20`

### Task 2.5: Implement `search` and `report` using SQL plus FTS5
- **Location**:
  - `crates/memo-cli/src/commands/search.rs`
  - `crates/memo-cli/src/commands/report.rs`
  - `crates/memo-cli/src/storage/search.rs`
- **Description**: Implement keyword search and summary reports with explicit SQL query paths and
  FTS-backed ranking, including weekly and monthly report windows.
- **Dependencies**:
  - Task 2.3
  - Task 2.4
- **Complexity**: 8
- **Acceptance criteria**:
  - `search` supports keyword and prefix terms with deterministic ranking tie-breakers.
  - `report` produces at least weekly and monthly summaries from stored records and derivations.
  - Query behavior remains functional when no derivation rows exist.
  - Integration tests verify representative search and report scenarios.
- **Validation**:
  - `cargo test -p nils-memo-cli search_and_report`
  - `cargo run -p nils-memo-cli -- search "tokyo"`
  - `cargo run -p nils-memo-cli -- report week`
  - `cargo run -p nils-memo-cli -- report month`

## Sprint 3: Agent integration, JSON guarantees, and shell UX
**Goal**: Make the CLI a stable API surface for agent scripts and automation.
**Demo/Validation**:
- Command(s): `cargo run -p nils-memo-cli -- fetch --json`
- Verify: unprocessed entries and apply results are round-trippable with stable JSON envelopes.

### Task 3.1: Implement `fetch` and `apply` derivation workflow
- **Location**:
  - `crates/memo-cli/src/commands/fetch.rs`
  - `crates/memo-cli/src/commands/apply.rs`
  - `crates/memo-cli/src/storage/derivations.rs`
- **Description**: Implement machine-facing commands for agent pull and write-back flows, with
  derivation versioning, active selection, confidence fields, and safe reprocessing behavior.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
  - Task 2.4
- **Complexity**: 8
- **Acceptance criteria**:
  - `fetch` returns only pending or requested records with deterministic ordering.
  - `apply` inserts a new derivation row and updates active selection safely in one transaction.
  - Existing derivations remain queryable for audit and rollback.
  - Tests cover repeated apply calls for the same item and active-derivation switching.
- **Validation**:
  - `cargo test -p nils-memo-cli fetch_apply_flow`
  - `cargo test -p nils-memo-cli apply_idempotency`
  - `cargo run -p nils-memo-cli -- fetch --json`
  - `cargo run -p nils-memo-cli -- apply --help`

### Task 3.2: Enforce JSON envelope and structured errors across commands
- **Location**:
  - `crates/memo-cli/src/output/json.rs`
  - `crates/memo-cli/src/errors.rs`
  - `crates/memo-cli/tests/json_contract.rs`
- **Description**: Centralize JSON serialization and error mapping so every JSON-capable command
  emits required envelope fields and stable machine error codes.
- **Dependencies**:
  - Task 1.3
  - Task 2.2
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - JSON output includes required envelope keys for success and failure responses.
  - Collection responses use `results` and single-item responses use `result`.
  - Errors expose stable `code` and concise `message`, with optional `details` fields.
  - Contract tests assert no sensitive value leakage in failure and success paths.
- **Validation**:
  - `cargo test -p nils-memo-cli json_contract`
  - `cargo test -p nils-memo-cli json_no_secret_leak`

### Task 3.3: Finalize human-readable renderer and color behavior
- **Location**:
  - `crates/memo-cli/src/output/text.rs`
  - `crates/memo-cli/src/output/mod.rs`
  - `crates/memo-cli/tests/text_output.rs`
- **Description**: Stabilize text output formatting, section ordering, and warning messaging while
  honoring `NO_COLOR=1` and preserving clean stdout for primary command results.
- **Dependencies**:
  - Task 2.4
  - Task 2.5
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Text output remains legible for capture, list, search, report, fetch, and apply flows.
  - `NO_COLOR=1` produces uncolored output without losing structural markers.
  - Warnings and diagnostics route to stderr, leaving stdout parse-friendly.
  - Snapshot or assertion tests protect formatting stability.
- **Validation**:
  - `cargo test -p nils-memo-cli text_output`
  - `NO_COLOR=1 cargo run -p nils-memo-cli -- list --limit 5`

### Task 3.4: Add shell completions and agent usage docs
- **Location**:
  - `completions/zsh/_memo-cli`
  - `completions/bash/memo-cli`
  - `crates/memo-cli/README.md`
  - `crates/memo-cli/docs/runbooks/memo-cli-agent-workflow.md`
- **Description**: Add completion support and a minimal runbook for agent orchestration with
  command examples, expected JSON payloads, and failure handling guidance.
- **Dependencies**:
  - Task 2.2
  - Task 3.1
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Zsh and Bash completions cover core subcommands and key flags.
  - README includes quick-start examples for capture, search, and report usage.
  - Runbook documents `fetch` and `apply` loop for automation scripts.
  - Completion regression tests pass without breaking existing completion suites.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "fetch|apply|search|report|--json" crates/memo-cli/docs/runbooks/memo-cli-agent-workflow.md crates/memo-cli/README.md`

## Sprint 4: End-to-end validation and release readiness
**Goal**: Prove reliability under repository delivery gates and document operational fallback.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: full required checks pass and MVP flow is reproducible from docs.

### Task 4.1: Build fixture-driven integration and regression tests
- **Location**:
  - `crates/memo-cli/tests/memo_flow.rs`
  - `crates/memo-cli/tests/agent_roundtrip.rs`
  - `crates/memo-cli/tests/fixtures/memo_seed.json`
- **Description**: Add integration tests that exercise capture, derivation apply, keyword search,
  and reporting, with reproducible fixture inputs and deterministic assertions.
- **Dependencies**:
  - Task 2.5
  - Task 3.1
  - Task 3.2
  - Task 3.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Integration suite covers add to fetch to apply to search to report flow.
  - Edge cases include empty dataset, malformed derivation payload, and duplicate apply attempts.
  - Test data remains repository-local and deterministic.
  - Failing cases report actionable assertion messages.
- **Validation**:
  - `cargo test -p nils-memo-cli memo_flow`
  - `cargo test -p nils-memo-cli agent_roundtrip`

### Task 4.2: Execute required repository checks and coverage gate
- **Location**:
  - `DEVELOPMENT.md`
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `scripts/ci/coverage-summary.sh`
- **Description**: Run required format, lint, and test checks, then execute coverage commands to
  verify repository-wide line coverage policy remains satisfied after adding the new crate.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Required checks entrypoint exits successfully.
  - Workspace tests and zsh completion tests remain green.
  - Coverage output meets or exceeds repository threshold.
  - Any check timing or flakiness concerns are documented for follow-up.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

### Task 4.3: Verify publishable-first release policy execution
- **Location**:
  - `crates/memo-cli/docs/specs/memo-cli-release-policy.md`
  - `crates/memo-cli/Cargo.toml`
  - `scripts/publish-crates.sh`
  - `release/crates-io-publish-order.txt`
- **Description**: Validate that publishable-first mode is correctly encoded and executable,
  including crates.io dry-run checks and release-order verification for `nils-memo-cli`.
- **Dependencies**:
  - Task 1.4
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Release policy document reflects actual crate metadata and repository wiring.
  - `nils-memo-cli` is present in release order and marked as publishable in policy docs.
  - Dry-run publish command is executable and succeeds for `nils-memo-cli`.
  - Validation outputs are recorded in PR summary notes.
- **Validation**:
  - `rg -n "publishable-first|crates.io|dry-run" crates/memo-cli/docs/specs/memo-cli-release-policy.md`
  - `bash -lc '! rg -n "^publish = false" crates/memo-cli/Cargo.toml'`
  - `rg -n "nils-memo-cli" release/crates-io-publish-order.txt`
  - `scripts/publish-crates.sh --dry-run --crate nils-memo-cli`

### Task 4.4: Document rollout and operational rollback guide
- **Location**:
  - `crates/memo-cli/docs/runbooks/memo-cli-rollout.md`
  - `crates/memo-cli/docs/runbooks/memo-cli-agent-workflow.md`
- **Description**: Write rollout steps, operational checks, and rollback triggers so teams can
  safely adopt the CLI for capture-first workflows and revert behavior if regressions appear.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
  - Task 4.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Rollout document includes setup, smoke test, and monitoring checkpoints.
  - Rollback section includes trigger criteria and concrete command-level rollback actions.
  - Agent workflow runbook includes fallback behavior when apply payload validation fails.
  - Documentation is linked from crate README for discoverability.
- **Validation**:
  - `rg -n "rollout|smoke|rollback|trigger|fetch|apply" crates/memo-cli/docs/runbooks/memo-cli-rollout.md crates/memo-cli/docs/runbooks/memo-cli-agent-workflow.md`
  - `rg -n "memo-cli-rollout|memo-cli-agent-workflow" crates/memo-cli/README.md`

## Dependency and parallelization map
- Critical path:
  - Task 1.1 -> Task 1.2 -> Task 2.3 -> Task 2.4 -> Task 2.5 -> Task 3.1 -> Task 3.2 -> Task 4.1
    -> Task 4.2 -> Task 4.4
- Parallelizable lanes:
  - Task 1.3 can run in parallel with Task 1.2 after Task 1.1 command semantics are fixed.
  - Task 2.2 can progress in parallel with Task 2.3 once Task 2.1 scaffold is complete.
  - Task 3.4 can run in parallel with Task 3.3 after Task 3.1 and Task 3.2 are stable.
  - Task 4.3 can run in parallel with Task 4.4 after Task 4.2 checks are complete.

## Testing Strategy
- Unit: parser validation, JSON serializer behavior, error-code mapping, SQL query builders, and
  derivation merge rules.
- Integration: command-level tests using temp SQLite databases for add/list/search/report/fetch/apply.
- E2E/manual: scripted flow using realistic memo inputs (shopping, travel date, book reminder) from
  capture through agent apply to report verification.

## Risks & gotchas
- FTS5 tokenizer and ranking choices may produce surprising search order on mixed-language content.
  Mitigation: lock tokenizer config and add deterministic tie-break sorting by id.
- Agent-provided derivation payloads can drift from schema expectations. Mitigation: strict JSON
  schema validation and explicit error codes for rejected payloads.
- SQLite write contention can appear if multiple agent processes run concurrently. Mitigation: WAL
  mode, short transactions, and single-writer guard recommendations in runbooks.
- Publishable-first delivery can fail late if metadata or release order drifts. Mitigation: enforce
  dry-run publish and release-order checks in required validation.

## Rollback plan
- Trigger criteria:
  - Data-loss risk discovered in capture path, or JSON contract breakage in automation consumers.
  - Required checks fail repeatedly due to storage or parser regressions after merge.
- Owner:
  - `nils-cli` maintainers responsible for `nils-memo-cli` crate and workflow docs.
- Rollback steps:
  - Revert `nils-memo-cli` command behavior changes to last stable commit while preserving database
    file compatibility.
  - Disable agent `apply` automation path and keep capture-only mode active if derivation issues
    occur.
  - Temporarily pin to text-mode user workflow and suspend JSON-based automation until contract
    tests pass again.
- Roll-forward criteria:
  - Re-enable full flow only after fixture-based integration tests, JSON contract tests, and
    required repository checks all return green.
