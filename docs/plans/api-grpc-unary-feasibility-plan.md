# Plan: gRPC Unary feasibility and delivery for api-grpc + api-test reuse

## Overview
This plan evaluates and delivers unary gRPC testing support in a way that mirrors existing `api-rest` / `api-gql` workflows and integrates into `api-test` with minimal contract drift. The highest-priority outcome is behavioral consistency: similar command shape, setup discovery, history/report behavior, and deterministic suite-runner artifacts. Feasibility is high: most `api-test` orchestration can remain unchanged, while protocol-specific work is isolated to a new `grpc` core module, a new `api-grpc` CLI crate, and additive suite schema/runner branches. The plan includes an explicit implementation decision gate for the unary transport backend and keeps streaming out of scope for this phase.

## Scope
- In scope:
  - Unary gRPC support only (single request -> single response).
  - New CLI crate `api-grpc` with parity-oriented commands (`call`, `history`, `report`, `report-from-cmd`).
  - Shared-core support in `api-testing-core` for gRPC request schema, execution, expectations, history/report rendering, and setup discovery.
  - Additive `api-test` suite-runner support for `type: "grpc"` cases.
  - Feasibility and reuse assessment written into implementation docs/specs.
- Out of scope:
  - gRPC streaming (`server`, `client`, `bidi`).
  - Automatic service discovery UX beyond unary MVP requirements.
  - Protocols other than gRPC.
  - Changes to REST/GraphQL user-facing defaults unrelated to gRPC support.

## Feasibility assessment (explicit)
1. Feasibility: **High** for unary gRPC.
2. `api-test` direct reuse: **High (~75-85% reusable paths)**.
   - Reusable mostly unchanged:
     - suite selection (`--suite`, `--suite-file`, tags/only/skip),
     - run directory + artifact layout,
     - result envelope (`summary`, `cases`, exit code mapping),
     - summary rendering + JUnit generation,
     - fail-fast and write gating framework.
   - Required additive changes:
     - suite schema defaults/case fields for gRPC,
     - runner dispatch branch for `type: grpc`,
     - gRPC-specific prepare/run helpers,
     - gRPC config resolution and token/endpoint resolution path.
3. Key risk: unary transport implementation choice (external `grpcurl` adapter vs native Rust dynamic invocation). This plan includes a decision gate before implementation commitment.

## Transport decision (implemented)
- **Transport decision**: unary MVP uses an external `grpcurl` adapter through `api-testing-core::grpc::runner`.
- **Selected**: Option A (`grpcurl`) for faster delivery, dynamic-proto ergonomics, and low integration churn with current request-file UX.
- **Rejected** for MVP: native Rust dynamic invocation (`tonic`/`prost-reflect` style runtime client).
  - Reason 1: materially higher implementation surface for dynamic descriptor loading and invocation.
  - Reason 2: more moving parts for parity-hardening and CI determinism in this sprint.
  - Reason 3: does not change the immediate suite-runner contract value for unary phase.
- Revisit conditions:
  - streaming scope starts (`server`/`client`/`bidi`),
  - `grpcurl` dependency becomes an operational blocker in target environments,
  - need for deeper transport-level telemetry than command adapter can provide cleanly.
- Runtime dependency impact:
  - documented in `BINARY_DEPENDENCIES.md` (`grpcurl` required for `api-grpc call` and suite `type: grpc` execution).

## CLI contract snapshot (implemented)
- Default command behavior:
  - `api-grpc <request.grpc.json>` is interpreted as `api-grpc call <request.grpc.json>`.
- Command surface:
  - `api-grpc call`
  - `api-grpc history`
  - `api-grpc report`
  - `api-grpc report-from-cmd`
- Shared parity-oriented flags:
  - endpoint: `--env`, `--url`
  - auth: `--token`
  - setup discovery: `--config-dir`
  - history control: `--no-history` (`call`)
- Exit/stdout/stderr conventions:
  - success call: response JSON/body on stdout, exit `0`.
  - command/runtime/expect failure: diagnostic on stderr, exit `1`.
  - suite run contract remains unchanged: any failed case => process exit `2`.
- Examples:
  - `api-grpc call --env local setup/grpc/requests/health.grpc.json`
  - `api-grpc report --case health --request setup/grpc/requests/health.grpc.json --run`
  - `api-grpc history --command-only | api-grpc report-from-cmd --stdin`

## Unary request schema snapshot (implemented)
- Required:
  - `method` (`service/method` form)
- Optional:
  - `body` (object, default `{}`)
  - `metadata` (string/scalar map)
  - `proto`, `importPaths[]`
  - `plaintext` (default `true`)
  - `authority`
  - `timeoutSeconds`
  - `expect.status`, `expect.jq`
- Example:
```json
{
  "method": "health.HealthService/Check",
  "body": {
    "service": "payments"
  },
  "metadata": {
    "x-trace-id": "demo-001"
  },
  "plaintext": true,
  "expect": {
    "status": 0,
    "jq": ".ok == true"
  }
}
```

## api-test reuse matrix (implemented evidence)
| Area | reuse matrix status | Implementation note | Validation evidence |
| --- | --- | --- | --- |
| Suite selection (`--suite`, `--suite-file`, tags/only/skip) | unchanged | no protocol-specific rewrite | `cargo test -p nils-api-testing-core --test suite_rest_graphql_matrix` |
| Run directory + artifact envelope | unchanged | existing run/result layout reused | `cargo test -p nils-api-testing-core --test suite_runner_loopback` |
| Results JSON / summary / JUnit contracts | unchanged | existing result serialization reused | `cargo test -p nils-api-testing-core suite::summary suite::junit suite::results` |
| Protocol dispatch | additive grpc | new `type: grpc` branch in suite runner | `cargo test -p nils-api-testing-core --test suite_runner_grpc_matrix` |
| Manifest defaults + case validation | additive grpc | `defaults.grpc` + grpc case checks | `cargo test -p nils-api-test suite_schema` |
| Endpoint override surface | additive grpc | `API_TEST_GRPC_URL` pass-through | `cargo test -p nils-api-testing-core suite::runtime_tests` |
| Streaming impact | out of scope (streaming) | reserved for later sprint/phase | documented here and in rollback section |

## Assumptions (if any)
1. gRPC unary command UX should follow existing CLIs (`api-rest`/`api-gql`) as closely as possible.
2. `setup/grpc` will be the canonical config directory for endpoint and token presets.
3. Auth/token profile behavior should mirror REST-style profile selection and env fallback semantics where practical.
4. Suite schema remains version `1` with additive fields; no breaking manifest migration in this phase.
5. Existing required checks in `DEVELOPMENT.md` remain mandatory for delivery.

## Sprint 1: Feasibility gate and contract freeze
**Goal**: Lock implementation direction, request schema, and CLI contract before code changes spread across crates.
**Demo/Validation**:
- Command(s):
  - `rg -n "Feasibility assessment|Transport decision|CLI contract|Suite contract" docs/plans/api-grpc-unary-feasibility-plan.md`
  - `rg -n "api-grpc|grpc" crates/api-testing-core/README.md crates/api-test/README.md || true`
- Verify:
  - Transport backend choice is recorded with tradeoffs.
  - CLI and suite contract drafts are explicit enough for implementation and tests.

### Task 1.1: Produce transport decision memo (unary execution backend)
- **Location**:
  - `docs/plans/api-grpc-unary-feasibility-plan.md`
  - `crates/api-testing-core/README.md`
- **Description**: Evaluate two unary execution backends and commit to one for MVP:
  - Option A: external `grpcurl` adapter (faster to ship, dynamic schema ergonomics, adds binary dependency).
  - Option B: native Rust invocation path (fewer external runtime dependencies, higher complexity for dynamic proto invocation).
  Capture decision criteria (complexity, determinism, CI portability, parity with existing design constraints).
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - One backend is selected for Sprint 2 implementation.
  - Rejected option is documented with concrete reasons and revisit conditions.
  - Runtime dependency impact is reflected in dependency docs if Option A is selected.
- **Validation**:
  - `rg -n "Transport decision|Selected|Rejected" docs/plans/api-grpc-unary-feasibility-plan.md`
  - `rg -n "grpcurl|tonic|native" docs/plans/api-grpc-unary-feasibility-plan.md`

### Task 1.2: Define `api-grpc` CLI surface (parity-first)
- **Location**:
  - `crates/api-testing-core/README.md`
  - `crates/api-test/README.md`
  - `docs/plans/api-grpc-unary-feasibility-plan.md`
- **Description**: Specify command and flag contracts for `api-grpc` with parity intent:
  - root + default `call`,
  - `history`,
  - `report`,
  - `report-from-cmd`,
  - shared flags pattern (`--env`, `--url`, `--token`, `--config-dir`, `--no-history`).
  Define output and exit-code rules aligned with existing API CLIs.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Contract includes concrete examples and edge-case behavior.
  - Exit-code and stdout/stderr conventions are explicit.
  - History/report parity expectations are listed.
- **Validation**:
  - `rg -n "api-grpc call|api-grpc history|api-grpc report|api-grpc report-from-cmd" docs/plans/api-grpc-unary-feasibility-plan.md`
  - `rg -n "exit code|stdout|stderr|Examples" docs/plans/api-grpc-unary-feasibility-plan.md`

### Task 1.3: Define gRPC unary request schema + fixture matrix
- **Location**:
  - `crates/api-testing-core/README.md`
  - `crates/api-test/README.md`
  - `docs/plans/api-grpc-unary-feasibility-plan.md`
- **Description**: Specify request-file schema and deterministic fixtures for unary:
  - service/method addressing,
  - message payload JSON,
  - metadata headers,
  - optional authority/TLS knobs (MVP-safe subset),
  - assertions (`expect.status`, `expect.jq` or equivalent assertion model).
  Define fixture cases for success, auth failure, schema mismatch, and assertion failure.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Request schema fields are fully specified.
  - Fixtures cover pass/fail and auth scenarios.
  - At least one fixture is reusable by `api-grpc` and `api-test`.
- **Validation**:
  - `rg -n "request schema|fixtures|assert" docs/plans/api-grpc-unary-feasibility-plan.md`

### Task 1.4: Define additive suite contract for `type: grpc`
- **Location**:
  - `crates/api-testing-core/src/suite/schema.rs`
  - `crates/api-test/src/suite_schema.rs`
  - `crates/api-test/README.md`
- **Description**: Specify additive suite-manifest fields:
  - `defaults.grpc` (`configDir`, `url`, `token`),
  - case-level gRPC fields (request path, token override, grpc-specific options),
  - validation rules and error messages for invalid gRPC cases.
  Keep schema version unchanged and backward compatible.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Existing REST/GraphQL suites remain valid with no changes.
  - New gRPC cases have deterministic validation failures for missing required fields.
  - Schema docs and examples include mixed REST/GraphQL/gRPC suites.
- **Validation**:
  - `rg -n "defaults\\.grpc|type: grpc|gRPC case" crates/api-test/README.md docs/plans/api-grpc-unary-feasibility-plan.md`

## Sprint 2: Shared core implementation for unary gRPC
**Goal**: Implement protocol core primitives in `api-testing-core` with clean boundaries and test coverage.
**Demo/Validation**:
- Command(s):
  - `cargo test -p api-testing-core grpc`
  - `cargo test -p api-testing-core suite_schema`
- Verify:
  - gRPC schema parsing/execution/assertions work in isolation.
  - Suite schema accepts valid gRPC cases and rejects invalid ones predictably.

### Task 2.1: Add `grpc` module to `api-testing-core`
- **Location**:
  - `crates/api-testing-core/src/lib.rs`
  - `crates/api-testing-core/src/grpc/mod.rs`
  - `crates/api-testing-core/src/grpc/schema.rs`
  - `crates/api-testing-core/src/grpc/runner.rs`
  - `crates/api-testing-core/src/grpc/expect.rs`
- **Description**: Implement unary request schema loader, execution adapter (per Sprint 1 decision), and assertion evaluator. Keep interfaces parallel to existing `rest`/`graphql` modules where useful.
- **Dependencies**:
  - Task 1.1
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Request schema loads with actionable errors.
  - Unary execution returns structured status/body/metadata result.
  - Assertions can mark pass/fail deterministically.
- **Validation**:
  - `cargo test -p api-testing-core grpc::schema`
  - `cargo test -p api-testing-core grpc::runner`
  - `cargo test -p api-testing-core grpc::expect`

### Task 2.2: Add gRPC setup discovery and token/endpoint resolution hooks
- **Location**:
  - `crates/api-testing-core/src/config.rs`
  - `crates/api-testing-core/src/cli_endpoint.rs`
  - `crates/api-testing-core/src/auth_env.rs`
- **Description**: Add `setup/grpc` discovery and endpoint/token resolution parity helpers:
  - endpoint presets from `endpoints.env(.local)`,
  - token profiles from `tokens.env(.local)`,
  - env fallback behavior aligned with current conventions.
- **Dependencies**:
  - Task 1.2
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - gRPC setup discovery is deterministic and testable.
  - Endpoint/token errors include available values where practical.
  - No regressions to REST/GraphQL resolution tests.
- **Validation**:
  - `cargo test -p api-testing-core config`
  - `cargo test -p api-testing-core cli_endpoint`
  - `cargo test -p api-testing-core auth_env`

### Task 2.3: Extend suite schema and runtime option surface for gRPC
- **Location**:
  - `crates/api-testing-core/src/suite/schema.rs`
  - `crates/api-test/src/suite_schema.rs`
  - `crates/api-testing-core/src/suite/runner/context.rs`
  - `crates/api-testing-core/src/suite/runtime.rs`
- **Description**: Add `defaults.grpc` and gRPC case fields to typed schema plus raw validator, then wire runner/runtime context to carry gRPC endpoint override env (for example `API_TEST_GRPC_URL`).
- **Dependencies**:
  - Task 1.4
  - Task 2.1
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Manifest validation supports mixed-protocol suites.
  - Runtime options include gRPC URL override without breaking existing env vars.
  - Schema-level tests cover positive + negative gRPC cases.
- **Validation**:
  - `cargo test -p api-testing-core suite::schema`
  - `cargo test -p api-test suite_schema`

### Task 2.4: Add suite runner gRPC branch and case artifact parity
- **Location**:
  - `crates/api-testing-core/src/suite/runner/mod.rs`
  - `crates/api-testing-core/src/suite/runner/grpc.rs`
  - `crates/api-testing-core/src/suite/results.rs`
- **Description**: Add `type: grpc` dispatch branch with prepare/run flow and artifact writing (`CASE_ID.response.json`, `CASE_ID.stderr.log`), while preserving summary/exit behavior.
- **Dependencies**:
  - Task 2.1
  - Task 2.3
- **Complexity**: 8
- **Acceptance criteria**:
  - gRPC cases execute in `api-test run` and produce standard artifact fields.
  - Fail-fast, tags/only/skip behavior remains unchanged for all protocols.
  - Existing results JSON contract remains backward compatible.
- **Validation**:
  - `cargo test -p api-testing-core suite::runner`
  - `cargo test -p api-testing-core suite::results`

## Sprint 3: `api-grpc` CLI crate (parity UX)
**Goal**: Deliver a user-facing unary gRPC CLI that looks and behaves like existing API CLIs.
**Demo/Validation**:
- Command(s):
  - `cargo run -p api-grpc -- --help`
  - `cargo run -p api-grpc -- call --help`
  - `cargo run -p api-grpc -- history --help`
  - `cargo run -p api-grpc -- report --help`
- Verify:
  - CLI surface, default command behavior, and help style match repo conventions.

### Task 3.1: Scaffold `api-grpc` crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/api-grpc/Cargo.toml`
  - `crates/api-grpc/src/main.rs`
  - `crates/api-grpc/src/cli.rs`
  - `crates/api-grpc/src/commands/mod.rs`
- **Description**: Add new crate with clap parsing, root help, default command insertion (`call`), and command dispatch consistent with `api-rest`/`api-gql`.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - `api-grpc` builds and exposes expected subcommands.
  - Root help and parse error behavior match existing API CLIs.
  - `-V/--version` works from root parser.
- **Validation**:
  - `cargo check -p api-grpc`
  - `cargo run -p api-grpc -- --help`

### Task 3.2: Implement `call` + `history`
- **Location**:
  - `crates/api-grpc/src/commands/call.rs`
  - `crates/api-grpc/src/commands/history.rs`
  - `crates/api-testing-core/src/cli_history.rs`
  - `crates/api-testing-core/src/history.rs`
- **Description**: Implement unary call execution with endpoint/token resolution, history write controls, and history reading semantics (`--last/--tail/--command-only`).
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - `call` executes unary request and prints response body to stdout.
  - History behavior mirrors REST/GraphQL command semantics.
  - Errors are printed to stderr with non-zero exit codes.
- **Validation**:
  - `cargo test -p api-grpc history`
  - `cargo test -p api-grpc call`

### Task 3.3: Implement `report` + `report-from-cmd`
- **Location**:
  - `crates/api-grpc/src/commands/report.rs`
  - `crates/api-grpc/src/commands/report_from_cmd.rs`
  - `crates/api-testing-core/src/report.rs`
  - `crates/api-testing-core/src/cli_report.rs`
- **Description**: Implement Markdown report generation with redaction and command snippet options aligned with existing CLIs.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Reports include request/response/assertion context and endpoint note.
  - `report-from-cmd` supports arg/stdin patterns and dry-run parity behavior.
  - Output path defaults follow existing report metadata conventions.
- **Validation**:
  - `cargo test -p api-grpc report`
  - `cargo test -p api-grpc report_from_cmd`

### Task 3.4: Add wrappers/completions/docs for `api-grpc`
- **Location**:
  - `wrappers/api-grpc`
  - `completions/zsh/_api-grpc`
  - `completions/bash/api-grpc`
  - `crates/api-grpc/README.md`
  - `crates/api-grpc/docs/README.md`
- **Description**: Add wrapper and shell completion entries, then document canonical setup/request/report workflows.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Wrapper and completion files exist and parse correctly.
  - README includes runnable quickstart and env/token examples.
  - Completion tests continue to pass.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "api-grpc" completions/zsh completions/bash crates/api-grpc/README.md`

## Sprint 4: `api-test` integration and end-to-end parity checks
**Goal**: Prove that `api-test` reuses existing orchestration and supports gRPC cases with minimal changes.
**Demo/Validation**:
- Command(s):
  - `cargo run -p api-test -- run --suite grpc-smoke`
  - `cargo test -p api-test`
  - `cargo test -p api-testing-core suite::runner`
- Verify:
  - gRPC cases run through the same suite pipeline and produce expected output artifacts.
  - Mixed-protocol suites remain deterministic and backward compatible.

### Task 4.1: Wire gRPC overrides and runner options into `api-test`
- **Location**:
  - `crates/api-test/src/main.rs`
  - `crates/api-testing-core/src/suite/runner/context.rs`
- **Description**: Add `API_TEST_GRPC_URL` wiring and pass-through options needed by gRPC runner path without changing existing REST/GraphQL contract behavior.
- **Dependencies**:
  - Task 2.3
  - Task 2.4
- **Complexity**: 5
- **Acceptance criteria**:
  - gRPC URL override is configurable from env.
  - Existing env overrides for REST/GraphQL remain unchanged.
  - No regressions in `api-test` CLI behavior.
- **Validation**:
  - `cargo test -p api-test cli_smoke`
  - `cargo test -p api-testing-core suite::runner`

### Task 4.2: Add deterministic gRPC fixtures and mixed-suite scenarios
- **Location**:
  - `crates/api-test/tests/grpc_integration.rs`
  - `crates/api-test/tests/fixtures/grpc/smoke.suite.json`
  - `crates/api-testing-core/tests/suite_runner_grpc_matrix.rs`
- **Description**: Add fixture servers and suites covering:
  - unary success,
  - auth/token failure,
  - assertion failure,
  - mixed REST + GraphQL + gRPC execution.
  Ensure test repos are self-contained and deterministic.
- **Dependencies**:
  - Task 2.4
  - Task 3.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Fixtures cover both success and failure exit mapping.
  - Mixed suite run validates summary counts and artifact paths.
  - Tests do not rely on machine-local external services.
- **Validation**:
  - `cargo test -p api-testing-core`
  - `cargo test -p api-test`

### Task 4.3: Verify and document api-test reuse boundaries
- **Location**:
  - `crates/api-test/README.md`
  - `crates/api-testing-core/README.md`
  - `docs/plans/api-grpc-unary-feasibility-plan.md`
- **Description**: Create an explicit reuse evidence matrix that maps each claimed reusable `api-test` path (suite selection, artifact layout, result envelope, summary/JUnit, fail-fast/write gating) to concrete validation commands and touched files. Document which components remained unchanged versus where additive gRPC logic was required, then include a short rationale for maintainability and future streaming extension.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Reuse boundary is explicit, traceable, and backed by command-level evidence.
  - Future streaming work impact surface is identified.
  - Docs are consistent across core + CLI README files.
- **Validation**:
  - `cargo test -p api-test`
  - `cargo test -p api-testing-core suite::summary suite::junit suite::results suite::filter`
  - `rg -n "reuse matrix|unchanged|additive grpc|streaming" crates/api-test/README.md crates/api-testing-core/README.md docs/plans/api-grpc-unary-feasibility-plan.md`

## Sprint 5: Hardening, release gates, and rollout safety
**Goal**: Ensure gRPC unary support meets repository quality gates and has an operational rollback path.
**Demo/Validation**:
- Command(s):
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
- Verify:
  - Required lint/test checks pass.
  - Coverage policy is met.
  - Documentation placement policy passes audit.

### Task 5.1: Run mandatory checks and fix regressions
- **Location**:
  - workspace-wide touched files
- **Description**: Run all required quality gates from `DEVELOPMENT.md`, resolve regressions, and capture key outcomes for release readiness.
- **Dependencies**:
  - Task 4.2
  - Task 4.3
- **Complexity**: 6
- **Acceptance criteria**:
  - `fmt`, `clippy`, workspace tests, zsh completion tests pass.
  - Coverage command meets threshold.
  - Failures (if any) are triaged with remediation notes.
- **Validation**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`

### Task 5.2: Final docs and dependency declaration audit
- **Location**:
  - `BINARY_DEPENDENCIES.md`
  - `crates/api-grpc/README.md`
  - `crates/api-testing-core/README.md`
  - `docs/specs/crate-docs-placement-policy.md` (reference only)
- **Description**: Update dependency and usage docs based on selected transport backend and ensure docs placement compliance.
- **Dependencies**:
  - Task 1.1
  - Task 5.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Runtime dependency changes are documented.
  - README usage examples match implemented flags.
  - Docs placement audit passes.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `rg -n "api-grpc|grpcurl|gRPC" BINARY_DEPENDENCIES.md crates/api-grpc/README.md crates/api-testing-core/README.md`

## Parallelization opportunities
- After Task 1.1 (transport decision), these can proceed in parallel with low overlap:
  - Task 2.2 (config/endpoint/token resolution) and Task 2.1 (grpc core module scaffold).
  - Task 3.1 (CLI scaffold) and Task 2.3 (suite schema extension).
- After Task 3.1, Task 3.2 (`call/history`) and Task 3.4 (completions/docs skeleton) can run in parallel.
- Task 4.2 fixture authoring can run in parallel with Task 4.1 option plumbing once Task 2.4 is merged.

## Testing Strategy
- Unit:
  - gRPC request schema parsing, endpoint/token resolution helpers, suite schema validation, assertion evaluation.
- Integration:
  - `api-grpc` command flows (`call`, `history`, `report`, `report-from-cmd`) with deterministic fixture servers.
  - `api-test` mixed-protocol suites including `type: grpc`.
- E2E/manual:
  - Local run in canonical repo layout (`setup/grpc`, `tests/api/suites`) with report/history artifact verification.

## Risks & gotchas
- Transport backend lock-in risk:
  - If unary implementation couples too tightly to one backend, later streaming/native migration cost rises.
  - Mitigation: keep an execution adapter boundary in `api-testing-core::grpc::runner`.
- Schema drift risk:
  - Ad-hoc gRPC fields could fragment suite manifest consistency.
  - Mitigation: enforce strict schema validation + fixture-based contract tests.
- Tooling/runtime dependency risk:
  - External backend (if selected) adds environment variability.
  - Mitigation: explicit dependency docs, startup preflight checks, deterministic CI fixtures.
- Behavioral parity risk:
  - Minor CLI UX mismatches can break existing mental model.
  - Mitigation: mirror `api-rest`/`api-gql` flag naming/help and reuse report/history helpers.

## Rollback plan
- If gRPC integration destabilizes suite runner:
  - Temporarily disable `type: grpc` dispatch in `api-test` while leaving REST/GraphQL unaffected.
  - Keep `api-grpc` crate behind a documented “experimental” status until checks are green.
- If transport backend causes operational failures:
  - Revert backend-specific adapter commits and retain schema/CLI scaffolding behind no-op guarded paths.
  - Restore previous workspace dependency footprint and keep docs explicit about unsupported gRPC execution.
- If coverage/check gates regress:
  - Revert gRPC-specific test-coupled changes in one patch set and re-introduce incrementally by sprint boundary.
