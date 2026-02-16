# Plan: API WebSocket CLI and suite support (`api-websocket` + `api-test`)

## Overview
This plan adds first-class WebSocket testing support to the existing API CLI family by introducing a new `api-websocket` crate and extending `api-testing-core` + `api-test` with additive WebSocket capabilities. The priority is consistency with current API CLIs: command shape, setup discovery, history/report behavior, suite artifacts, and deterministic exit codes. Because no legacy `api-websocket` shell baseline exists in this repository, parity is defined against the existing Rust API CLI conventions (`api-rest`, `api-gql`, `api-grpc`, `api-test`) and the workspace CLI standards. Delivery is staged so existing REST/GraphQL/gRPC behavior remains unchanged while WebSocket support is added incrementally.

## Scope
- In scope:
  - New publishable CLI crate `nils-api-websocket` (`api-websocket` binary).
  - New `api-testing-core::websocket` module for request schema loading, transport execution, assertions, history/report helpers, and command-snippet integration.
  - Additive `api-test` support for suite case `type: "websocket"` with deterministic artifacts and summary/JUnit compatibility.
  - Setup discovery + auth conventions for `setup/websocket` with endpoint/token env files and profile selection.
  - Completion/wrapper/docs/dependency updates aligned with existing API CLI patterns.
- Out of scope:
  - WebSocket server implementation or a general-purpose local WebSocket mocking framework.
  - Cross-process streaming/session orchestration beyond deterministic scripted case execution.
  - Binary-frame heavy workflows (MVP focuses on text/JSON-first validation).
  - Changes to REST/GraphQL/gRPC command defaults not required for additive WebSocket support.

## Assumptions (if any)
1. No existing `api-websocket` script contract exists; behavioral baseline is the existing Rust API CLI family and workspace standards.
2. Canonical setup directory is `setup/websocket`.
3. Initial auth/profile model mirrors REST/gRPC (`--token`, `WS_TOKEN_NAME`, env fallback where allowed).
4. Initial suite override env var is `API_TEST_WS_URL` (additive; existing override vars remain unchanged).
5. `api-websocket` should include explicit machine-readable mode (`--format json`) for service-consumed output contracts.

## Sprint 1: Contract freeze and architecture decisions
**Goal**: Lock transport, naming, schema, and command contracts before implementation spreads across crates.
**Demo/Validation**:
- Command(s):
  - `rg -n "api-websocket|websocket|API_TEST_WS_URL|WS_URL_|WS_TOKEN_" docs/plans/api-websocket-cli-plan.md`
  - `rg -n "JSON Contract|schema_version|error.code" docs/specs/cli-service-json-contract-guideline-v1.md`
- Verify:
  - Transport/runtime dependency policy is explicit.
  - Setup naming and request/suite contract decisions are concrete enough to implement without rework.
**Parallelizable**: After Task 1.1, Tasks 1.2 and 1.3 can run in parallel.

### Task 1.1: Decide transport backend and runtime dependency policy
- **Location**:
  - `docs/plans/api-websocket-cli-plan.md`
  - `BINARY_DEPENDENCIES.md`
  - `crates/api-testing-core/README.md`
- **Description**: Decide between pure-Rust transport (`tokio-tungstenite` style) and external adapter execution (`websocat` style). Record selected backend, rejected option, and revisit conditions. Document runtime dependency impact and platform considerations.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - One transport backend is selected for MVP.
  - Rejected backend includes concrete tradeoffs and revisit triggers.
  - Runtime dependency documentation is updated (if external adapter is selected).
- **Validation**:
  - `rg -n "Selected backend:|Rejected backend:|Revisit when:" crates/api-websocket/README.md`
  - `rg -n "Runtime dependency policy|runtime dependency" crates/api-websocket/README.md BINARY_DEPENDENCIES.md`

### Task 1.2: Define setup discovery and naming conventions
- **Location**:
  - `crates/api-testing-core/README.md`
  - `crates/api-websocket/README.md`
  - `docs/plans/api-websocket-cli-plan.md`
- **Description**: Define canonical discovery and naming contracts for `setup/websocket`, endpoint/token env prefixes (`WS_URL_`, `WS_TOKEN_`), default endpoint behavior, and history filename (`.ws_history`) parity with existing API CLIs.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Setup dir, env var names, and history path are explicitly documented.
  - Precedence order for URL/token resolution is explicitly documented.
  - Naming conventions are consistent with existing API CLI family patterns.
- **Validation**:
  - `rg -n "setup/websocket|WS_URL_|WS_TOKEN_|.ws_history|precedence" crates/api-testing-core/README.md crates/api-websocket/README.md docs/plans/api-websocket-cli-plan.md`

### Task 1.3: Define WebSocket request schema v1 and fixture matrix
- **Location**:
  - `crates/api-websocket/docs/specs/websocket-request-schema-v1.md`
  - `crates/api-websocket/README.md`
  - `crates/api-testing-core/README.md`
- **Description**: Specify request-file schema for deterministic scripted sessions (connect parameters, ordered send/receive steps, timeout controls, close behavior, and assertion expressions). Define fixture matrix for handshake failure, timeout, assertion failure, and success paths.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Request schema fields and defaults are fully specified.
  - Fixture matrix covers success and key failure classes.
  - At least one fixture pattern is reusable by `api-websocket` and `api-test`.
- **Validation**:
  - `rg -n "request schema|connect|send|receive|timeout|assert|fixture" crates/api-websocket/docs/specs/websocket-request-schema-v1.md crates/api-websocket/README.md`

### Task 1.4: Define CLI contract and JSON output contract
- **Location**:
  - `crates/api-websocket/README.md`
  - `crates/api-websocket/docs/specs/websocket-cli-contract-v1.md`
  - `docs/specs/cli-service-json-contract-guideline-v1.md`
- **Description**: Define command surface (`call`, `history`, `report`, `report-from-cmd`), exit codes, stdout/stderr behavior, and machine-readable output (`--format json`) including versioned envelope and stable error codes.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - CLI flags and defaults are explicit and parity-aligned with existing API CLIs.
  - JSON envelope fields (`schema_version`, `command`, `ok`, `result/results`, `error`) are explicit.
  - Error code taxonomy is documented with stable machine-facing codes.
- **Validation**:
  - `rg -n "call|history|report|report-from-cmd|--format json|schema_version|error.code" crates/api-websocket/README.md crates/api-websocket/docs/specs/websocket-cli-contract-v1.md`

### Task 1.5: Define additive suite contract for `type: websocket`
- **Location**:
  - `crates/api-testing-core/src/suite/schema.rs`
  - `crates/api-test/src/suite_schema.rs`
  - `crates/api-test/README.md`
- **Description**: Define additive suite fields for WebSocket defaults and case-level overrides (`defaults.websocket`, `type: "websocket"`, request path, URL/token/config override semantics). Ensure backward compatibility for existing suite types.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Existing suite manifests remain valid and unchanged.
  - Missing required WebSocket fields fail with deterministic schema errors.
  - README includes at least one mixed-protocol suite example containing `websocket`.
- **Validation**:
  - `rg -n "defaults\\.websocket|type: \"websocket\"|websocket case" crates/api-test/README.md docs/plans/api-websocket-cli-plan.md`

## Sprint 2: Shared core WebSocket module in `api-testing-core`
**Goal**: Implement reusable core WebSocket schema/runner/assertion primitives with deterministic tests.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-api-testing-core websocket::schema`
  - `cargo test -p nils-api-testing-core websocket::runner`
  - `cargo test -p nils-api-testing-core websocket::expect`
- Verify:
  - Request files parse with actionable errors.
  - Scripted WebSocket sessions execute deterministically with reproducible assertions and logs.
**Parallelizable**: After Task 2.1, Tasks 2.2 and 2.4 can run in parallel.

### Task 2.1: Add `websocket` core module scaffold and schema loader
- **Location**:
  - `crates/api-testing-core/src/lib.rs`
  - `crates/api-testing-core/src/websocket/mod.rs`
  - `crates/api-testing-core/src/websocket/schema.rs`
- **Description**: Add the new module and request schema parser with strict validation and human-readable error messages aligned with other protocol modules.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - `api-testing-core` exports `websocket` module.
  - Schema parsing rejects malformed or incomplete request files with deterministic errors.
  - Unit tests cover required/optional field handling.
- **Validation**:
  - `cargo test -p nils-api-testing-core websocket::schema`

### Task 2.2: Implement transport runner for scripted session execution
- **Location**:
  - `crates/api-testing-core/src/websocket/runner.rs`
  - `crates/api-testing-core/src/websocket/transport.rs`
  - `crates/api-testing-core/src/http.rs`
- **Description**: Implement connection lifecycle, ordered send/receive steps, timeout handling, and deterministic close semantics using the selected Sprint 1 transport backend.
- **Dependencies**:
  - Task 1.1
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Runner supports deterministic scripted exchange and explicit timeout failures.
  - Connection/IO errors map to stable failure categories.
  - Transport abstraction boundary is explicit to contain backend-specific logic.
- **Validation**:
  - `cargo test -p nils-api-testing-core websocket::runner`

### Task 2.3: Implement assertion evaluation and transcript projection
- **Location**:
  - `crates/api-testing-core/src/websocket/expect.rs`
  - `crates/api-testing-core/src/websocket/transcript.rs`
  - `crates/api-testing-core/src/websocket/report.rs`
- **Description**: Implement assertion evaluation for receive steps and projected transcript output suitable for report rendering and suite artifacts.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Assertions support deterministic pass/fail reasoning with useful diagnostics.
  - Transcript projection is stable and redaction-aware.
  - Report-facing structures are reusable by `api-websocket` and `api-test`.
- **Validation**:
  - `cargo test -p nils-api-testing-core websocket::expect`
  - `cargo test -p nils-api-testing-core websocket::report`

### Task 2.4: Integrate setup discovery, endpoint/token resolution, and history path helpers
- **Location**:
  - `crates/api-testing-core/src/config.rs`
  - `crates/api-testing-core/src/cli_endpoint.rs`
  - `crates/api-testing-core/src/env_file.rs`
  - `crates/api-testing-core/src/history.rs`
- **Description**: Add setup resolution and env/profile lookup helpers for WebSocket protocol parity (`setup/websocket`, `WS_URL_*`, `WS_TOKEN_*`) plus protocol-specific history file defaults.
- **Dependencies**:
  - Task 1.2
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - URL/token lookup precedence matches documented policy.
  - Missing profile errors list available profiles where possible.
  - History file defaults and rotation policy hooks mirror existing API CLI behavior.
- **Validation**:
  - `cargo test -p nils-api-testing-core config::`
  - `cargo test -p nils-api-testing-core suite::runtime_tests`

### Task 2.5: Extend command-snippet parsing for WebSocket report replay
- **Location**:
  - `crates/api-testing-core/src/cmd_snippet.rs`
  - `crates/api-testing-core/src/websocket/report.rs`
- **Description**: Add parser support for `api-websocket call ...` snippets so `report-from-cmd` can reconstruct report inputs with parity behavior.
- **Dependencies**:
  - Task 2.3
  - Task 2.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Valid WebSocket snippets parse into structured report-from-cmd inputs.
  - Invalid snippets fail with protocol-specific guidance.
  - Existing REST/GraphQL/gRPC snippet parsing remains unchanged.
- **Validation**:
  - `cargo test -p nils-api-testing-core cmd_snippet::`

## Sprint 3: New `api-websocket` CLI crate and UX surfaces
**Goal**: Deliver a publishable CLI crate with parity command surface, JSON contract, tests, wrappers, and completions.
**Demo/Validation**:
- Command(s):
  - `cargo run -p nils-api-websocket -- --help`
  - `cargo run -p nils-api-websocket -- call --help`
  - `cargo test -p nils-api-websocket`
- Verify:
  - CLI help/flags/default-command behavior are stable.
  - Call/history/report/report-from-cmd work against deterministic fixtures.
**Parallelizable**: After Task 3.1, Tasks 3.2 and 3.5 can run in parallel.

### Task 3.1: Scaffold crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/api-websocket/Cargo.toml`
  - `crates/api-websocket/src/main.rs`
  - `crates/api-websocket/README.md`
- **Description**: Create publishable crate metadata, workspace membership, binary target wiring, and README skeleton aligned with workspace crate standards.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - `nils-api-websocket` is present in workspace metadata and builds.
  - Root parser supports `-V/--version`.
  - README includes command overview and setup conventions.
- **Validation**:
  - `cargo check -p nils-api-websocket`
  - `cargo run -p nils-api-websocket -- -V`
  - `cargo metadata --no-deps | rg "\"name\": \"nils-api-websocket\""`

### Task 3.2: Implement CLI parsing and default-command entry behavior
- **Location**:
  - `crates/api-websocket/src/main.rs`
  - `crates/api-websocket/src/cli.rs`
- **Description**: Implement clap command tree (`call`, `history`, `report`, `report-from-cmd`) with root help and default-command insertion behavior matching existing API CLIs.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Root help and subcommand help output are deterministic.
  - Bare request path invocation maps to `call`.
  - Invalid usage exits with stable non-zero behavior.
- **Validation**:
  - `cargo run -p nils-api-websocket -- --help`
  - `cargo run -p nils-api-websocket -- report --help`

### Task 3.3: Implement command handlers using shared core
- **Location**:
  - `crates/api-websocket/src/commands/mod.rs`
  - `crates/api-websocket/src/commands/call.rs`
  - `crates/api-websocket/src/commands/history.rs`
  - `crates/api-websocket/src/commands/report.rs`
- **Description**: Implement command behavior using `api-testing-core::websocket` primitives, including history writing, report generation, and report-from-cmd replay.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
  - Task 2.4
  - Task 2.5
  - Task 3.2
- **Complexity**: 8
- **Acceptance criteria**:
  - `call` prints expected response/transcript output and returns stable exit codes.
  - `history` tail/command-only behavior matches existing API CLI conventions.
  - `report` and `report-from-cmd` produce deterministic markdown outputs.
- **Validation**:
  - `cargo test -p nils-api-websocket --test integration`

### Task 3.4: Add machine-readable JSON mode and structured error envelope
- **Location**:
  - `crates/api-websocket/src/cli.rs`
  - `crates/api-websocket/src/commands/call.rs`
  - `crates/api-websocket/src/commands/history.rs`
  - `crates/api-websocket/tests/json_contract.rs`
  - `crates/api-websocket/docs/specs/websocket-cli-contract-v1.md`
- **Description**: Implement `--format json` output with guideline-compliant envelope and stable machine-readable error codes while keeping human-readable output as default.
- **Dependencies**:
  - Task 1.4
  - Task 3.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Success and failure JSON responses include required envelope keys.
  - Error payload uses stable `error.code` with optional structured details.
  - No token/secret material appears in JSON output paths.
- **Validation**:
  - `cargo test -p nils-api-websocket json_contract`
  - `rg -n "schema_version|error\\.code|--format json" crates/api-websocket/README.md crates/api-websocket/docs/specs/websocket-cli-contract-v1.md`

### Task 3.5: Add wrapper and shell completions
- **Location**:
  - `wrappers/api-websocket`
  - `completions/zsh/_api-websocket`
  - `completions/bash/api-websocket`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add wrapper script and shell completion files aligned with API CLI conventions, plus completion test updates.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Wrapper supports auto/debug/installed modes consistent with sibling wrappers.
  - Zsh/Bash completions include all major subcommands and key flags.
  - Completion regression tests cover the new binary.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "api-websocket" completions/zsh/_api-websocket completions/bash/api-websocket wrappers/api-websocket`

### Task 3.6: Add CLI smoke and integration tests
- **Location**:
  - `crates/api-websocket/tests/cli_smoke.rs`
  - `crates/api-websocket/tests/integration.rs`
- **Description**: Add deterministic tests for help/usage/errors plus scripted WebSocket call/report/history behaviors using local fixtures.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Smoke tests lock CLI surface and usage errors.
  - Integration tests cover success + failure + timeout + assertion mismatch.
  - Test suite does not rely on external network services.
- **Validation**:
  - `cargo test -p nils-api-websocket --test cli_smoke`
  - `cargo test -p nils-api-websocket --test integration`

## Sprint 4: `api-test` suite integration for WebSocket cases
**Goal**: Additive suite support for `type: websocket` with stable artifacts, summaries, and compatibility.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-api-testing-core --test suite_runner_websocket_matrix`
  - `cargo test -p nils-api-test suite_schema`
  - `cargo run -p nils-api-test -- run --suite-file tests/api/suites/websocket-smoke.suite.json`
- Verify:
  - WebSocket suites execute deterministically.
  - Existing REST/GraphQL/gRPC suite behavior remains unchanged.
**Parallelizable**: After Task 4.1, Tasks 4.2 and 4.3 can run in parallel.

### Task 4.1: Extend suite schema and validation for WebSocket defaults/cases
- **Location**:
  - `crates/api-testing-core/src/suite/schema.rs`
  - `crates/api-test/src/suite_schema.rs`
  - `crates/api-test/tests/suite_schema.rs`
- **Description**: Add additive manifest fields for WebSocket defaults and case-level overrides, plus deterministic validation errors for missing/invalid fields.
- **Dependencies**:
  - Task 1.5
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Backward compatibility for existing suite files is preserved.
  - `type: websocket` without required fields is rejected deterministically.
  - Validation tests include both valid and invalid websocket cases.
- **Validation**:
  - `cargo test -p nils-api-test suite_schema`
  - `cargo test -p nils-api-testing-core suite::schema`

### Task 4.2: Add runtime URL/token resolution hooks for suite execution
- **Location**:
  - `crates/api-testing-core/src/suite/runtime.rs`
  - `crates/api-testing-core/src/suite/runtime_tests.rs`
  - `crates/api-testing-core/src/suite/runner/context.rs`
  - `crates/api-test/src/main.rs`
- **Description**: Add WebSocket runtime resolution with precedence parity and new env override `API_TEST_WS_URL` for suite-wide target control.
- **Dependencies**:
  - Task 2.4
  - Task 4.1
- **Complexity**: 6
- **Acceptance criteria**:
  - WebSocket URL/token resolution precedence matches documented behavior.
  - `API_TEST_WS_URL` overrides per-suite defaults when set.
  - Runtime tests cover override/default/env-file branches.
- **Validation**:
  - `cargo test -p nils-api-testing-core suite::runtime_tests`
  - `rg -n "API_TEST_WS_URL|resolve_ws|websocket" crates/api-test/src/main.rs crates/api-testing-core/src/suite/runtime.rs`

### Task 4.3: Implement suite runner branch for `type: websocket`
- **Location**:
  - `crates/api-testing-core/src/suite/runner/mod.rs`
  - `crates/api-testing-core/src/suite/runner/websocket.rs`
  - `crates/api-testing-core/src/suite/results.rs`
- **Description**: Add case preparation/execution path for websocket suite cases, including stdout/stderr artifact files, command snippets, and status mapping (`passed/failed/skipped`).
- **Dependencies**:
  - Task 2.2
  - Task 2.3
  - Task 4.1
  - Task 4.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Suite runner executes websocket cases and writes deterministic artifacts.
  - Fail-fast and filtering semantics remain consistent.
  - Mixed protocol suites run without changing existing case behavior.
- **Validation**:
  - `cargo test -p nils-api-testing-core --test suite_runner_websocket_matrix`
  - `cargo test -p nils-api-testing-core --test suite_runner_loopback`

### Task 4.4: Add WebSocket suite fixtures and mixed-protocol matrix tests
- **Location**:
  - `crates/api-testing-core/tests/suite_runner_websocket_matrix.rs`
  - `crates/api-testing-core/tests/suite_rest_graphql_matrix.rs`
  - `crates/api-test/README.md`
- **Description**: Add fixtures and tests for websocket-only and mixed REST/GraphQL/gRPC/WebSocket suites with deterministic outputs and summary counts.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 7
- **Acceptance criteria**:
  - WebSocket-only suite pass/fail cases are covered.
  - Mixed suite confirms protocol coexistence and summary correctness.
  - Artifacts include expected response/log file mappings.
- **Validation**:
  - `cargo test -p nils-api-testing-core --test suite_runner_websocket_matrix`
  - `cargo test -p nils-api-testing-core --test suite_rest_graphql_matrix`

### Task 4.5: Update suite docs and contract examples
- **Location**:
  - `crates/api-test/README.md`
  - `crates/api-testing-core/README.md`
- **Description**: Document `type: websocket` contract, default setup conventions, env override behavior, and mixed-suite examples.
- **Dependencies**:
  - Task 4.4
- **Complexity**: 4
- **Acceptance criteria**:
  - README documents additive websocket suite fields and examples.
  - Existing examples remain valid and unchanged for other protocols.
  - Env override table includes `API_TEST_WS_URL`.
- **Validation**:
  - `rg -n "websocket|API_TEST_WS_URL|mixed protocol" crates/api-test/README.md crates/api-testing-core/README.md`

## Sprint 5: Delivery hardening, checks, and rollout safety
**Goal**: Complete repository gates, publish-readiness metadata, and operational rollback/runbook coverage.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
- Verify:
  - Required quality gates pass.
  - Documentation placement and coverage policy pass.
  - New crate is publishable and release-order documented.
**Parallelizable**: Task 5.1 can run in parallel with late-stage testing fixes from Task 5.2.

### Task 5.1: Final documentation and dependency inventory updates
- **Location**:
  - `BINARY_DEPENDENCIES.md`
  - `crates/api-websocket/README.md`
  - `crates/api-testing-core/README.md`
  - `crates/api-test/README.md`
- **Description**: Finalize protocol docs, dependency notes, setup examples, and contract references after implementation details stabilize.
- **Dependencies**:
  - Task 3.5
  - Task 4.5
- **Complexity**: 4
- **Acceptance criteria**:
  - Runtime dependency requirements are explicit.
  - Setup and command examples are runnable and aligned with implemented flags.
  - Cross-doc references are consistent and non-duplicative.
- **Validation**:
  - `rg -n "api-websocket|websocket|setup/websocket|dependency" BINARY_DEPENDENCIES.md crates/api-websocket/README.md crates/api-testing-core/README.md crates/api-test/README.md`

### Task 5.2: Run mandatory repository checks and coverage gate
- **Location**:
  - `DEVELOPMENT.md`
  - `tests/zsh/completion.test.zsh`
  - `target/coverage/lcov.info`
- **Description**: Run required workspace checks and coverage threshold, triage failures, and apply fixes until all mandatory gates pass.
- **Dependencies**:
  - Task 3.6
  - Task 4.4
  - Task 5.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Workspace fmt/clippy/tests/completion checks pass.
  - Coverage command passes fail-under policy.
  - Any failures are documented with actionable remediation.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`

### Task 5.3: Publish-readiness wiring for the new crate
- **Location**:
  - `release/crates-io-publish-order.txt`
  - `crates/api-websocket/Cargo.toml`
  - `scripts/publish-crates.sh`
- **Description**: Add `nils-api-websocket` to publish order in a dependency-safe position and verify publish dry-run succeeds.
- **Dependencies**:
  - Task 3.1
  - Task 5.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Publish order includes `nils-api-websocket`.
  - Crate metadata is publish-ready and consistent with workspace standards.
  - Publish dry-run passes for the new crate.
- **Validation**:
  - `scripts/publish-crates.sh --dry-run --crate nils-api-websocket`
  - `rg -n "nils-api-websocket" release/crates-io-publish-order.txt crates/api-websocket/Cargo.toml`

### Task 5.4: Rollout/runbook and operational fallback guide
- **Location**:
  - `crates/api-websocket/docs/runbooks/api-websocket-rollout.md`
  - `crates/api-test/docs/runbooks/api-test-websocket-adoption.md`
- **Description**: Write rollout playbook for adopting websocket tests gradually, with failure triage, timeout tuning guidance, and explicit fallback steps.
- **Dependencies**:
  - Task 5.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Runbook includes phased rollout and rollback triggers.
  - Troubleshooting section covers handshake/auth/timeout/assertion failures.
  - Guidance is actionable for both local and CI runs.
- **Validation**:
  - `rg -n "rollout|rollback|timeout|auth|troubleshooting" crates/api-websocket/docs/runbooks/api-websocket-rollout.md crates/api-test/docs/runbooks/api-test-websocket-adoption.md`

## Parallelization opportunities
- After Task 1.1, contract work can split between setup/auth naming (Task 1.2) and request schema design (Task 1.3).
- After Task 2.1, transport runner (Task 2.2) and config/history resolution (Task 2.4) can proceed in parallel with minimal file overlap.
- After Task 3.1, CLI parser work (Task 3.2) and completion/wrapper work (Task 3.5) can run in parallel.
- After Task 4.1, runtime override wiring (Task 4.2) and runner branch implementation scaffold (Task 4.3) can run concurrently, then converge in matrix tests (Task 4.4).
- Final docs polishing (Task 5.1) can run in parallel with stabilization fixes discovered during mandatory checks (Task 5.2).

## Testing Strategy
- Unit:
  - WebSocket schema parsing, step validation, assertion evaluation, URL/token resolution precedence, and JSON contract envelope validation.
- Integration:
  - `api-websocket` command flows (`call`, `history`, `report`, `report-from-cmd`) with deterministic local fixtures.
  - `api-test` websocket-only and mixed-protocol suites.
- E2E/manual:
  - Run against representative local WebSocket endpoint(s) with auth/no-auth, timeout, and failure-path checks.
  - Validate generated reports and suite artifacts under `out/api-test-runner/<run_id>/`.

## Risks & gotchas
- Timing/flakiness risk:
  - WebSocket tests can become flaky when receive ordering and timeout budgets are underspecified.
  - Mitigation: deterministic scripted steps, explicit per-step timeout defaults, and stable fixture servers.
- Contract drift risk:
  - Adding JSON mode in one protocol can diverge from the API CLI family if envelope/error contracts are inconsistent.
  - Mitigation: enforce guideline-based contract tests and shared helper usage.
- Secret leakage risk:
  - Transcript and JSON outputs may inadvertently expose auth material.
  - Mitigation: redaction-by-default and explicit leakage tests in success/failure paths.
- Suite compatibility risk:
  - New `type: websocket` logic could accidentally affect existing protocol routing.
  - Mitigation: keep additive branching and re-run existing suite matrix tests.
- Operational dependency risk:
  - Transport backend/runtime assumptions may vary across CI and developer environments.
  - Mitigation: document dependency requirements and keep transport boundary isolated for future backend swaps.

## Rollback plan
- If WebSocket suite integration regresses stability:
  - Temporarily disable `type: websocket` execution path in `api-test` while keeping REST/GraphQL/gRPC paths intact.
  - Keep schema/docs marked as experimental until regression root cause is fixed.
- If `api-websocket` command behavior is unstable:
  - Gate new subcommand behaviors behind conservative defaults and disable risky flags in a patch release.
  - Retain read-only/report capabilities while mutating/session-heavy flows are hardened.
- If transport/backend choice proves operationally unreliable:
  - Revert backend-specific runner integration while keeping request schema + CLI scaffolding.
  - Restore prior dependency footprint and document temporary unsupported status.
- If mandatory repo gates fail near release:
  - Revert websocket-specific commit slices by sprint boundary (core, CLI, suite, docs) to restore green baseline quickly.
