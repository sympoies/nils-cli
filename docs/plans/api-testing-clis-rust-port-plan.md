# Plan: Rust API testing CLIs parity (api-rest, api-gql, api-test)

## Overview
This plan ports the existing Bash-based API testing tooling (REST runner, GraphQL runner, and suite runner)
into three Rust CLI binaries inside this workspace: `api-rest`, `api-gql`, and `api-test`.
The goal is behavioral parity with the current scripts (flags, defaults, exit codes, history/report behavior),
while improving determinism and portability by moving core logic into Rust (and away from shell glue).
The outcome is a cohesive set of CLIs with shared core libraries, comprehensive fixtures, and CI-friendly tests.

## Scope
- In scope: three Rust binaries (`api-rest`, `api-gql`, `api-test`), shared core crate, parity docs (`spec`/`fixtures`),
  report generation parity, suite-runner parity (JSON results + optional JUnit), and comprehensive tests.
- Out of scope: updating the upstream Codex Kit skill scripts in-place, changing user-facing defaults/UX beyond parity,
  adding new protocol support beyond REST and GraphQL, or building a full HTTP mocking framework.

## Assumptions (if any)
1. The source-of-truth behavior is the current scripts and their referenced contracts:
   - `rest.sh`, `rest-history.sh`, `rest-report.sh`
   - `gql.sh`, `gql-history.sh`, `gql-report.sh`, `gql-schema.sh`
   - `api-test.sh`, `api-test-summary.sh`
2. CLI flags and environment variable names should remain compatible where reasonable (REST_* and GQL_* prefixes, suite flags).
3. Tests must be deterministic: they should spin up local HTTP servers and avoid relying on user machine state.
4. The Rust implementation should not print secrets by default and should preserve redaction behavior in reports.
5. This repo remains a multi-binary Rust workspace; each CLI is its own crate, with a shared core library crate.

## Sprint 1: Parity specs, fixtures, and dependency decisions
**Goal**: Make current behavior explicit and lock down a parity contract for each CLI (plus shared conventions).
**Demo/Validation**:
- Command(s): See https://github.com/graysurf/agent-kit/tree/main/skills/tools/testing
- Verify: Specs capture flags, env vars, exit codes, history/report semantics, and degradation paths.

### Task 1.1: Write shared overview and CLI mapping
- **Location**:
  - `crates/api-testing-core/README.md`
- **Description**: Document how the three binaries map to the existing scripts, the shared conventions (config discovery,
  path resolution rules, output rules, secret handling), and what is considered parity-critical vs best-effort.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Overview clearly maps each legacy script to a Rust subcommand or mode.
  - Overview defines consistent terminology: setup dir, config dir, env preset, token profile, history, report.
  - Overview lists the canonical repo layouts supported by the suite runner (setup/ and tests/).
- **Validation**:
  - `rg -n "^# " crates/api-testing-core/README.md`
  - `rg -n "api-rest|api-gql|api-test" crates/api-testing-core/README.md`

### Task 1.2: Write api-rest parity spec
- **Location**:
  - `crates/api-rest/README.md`
- **Description**: Specify `api-rest` CLI behavior based on `rest.sh` and its report/history scripts: arguments, env vars,
  request JSON schema, auth selection rules, JWT validation behavior, exit codes, and output conventions.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Spec includes CLI surface (flags, env vars, help text expectations) and exit code contract.
  - Spec captures request schema rules (method/path/query/headers/body/multipart/cleanup/expect).
  - Spec includes history semantics and report semantics (including redaction defaults).
  - Spec includes an explicit external-dependency inventory for api-rest (and the chosen policy per dependency).
- **Validation**:
  - `rg -n "^# api-rest parity spec" crates/api-rest/README.md`
  - `rg -n "Request schema" crates/api-rest/README.md`
  - `rg -n "External dependencies" crates/api-rest/README.md`

### Task 1.3: Write api-rest fixtures
- **Location**:
  - `crates/api-rest/README.md`
- **Description**: Define deterministic fixture scenarios for `api-rest`, including successful calls, expect failures,
  multipart upload, cleanup templating, and history/report generation.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Fixtures cover success and failure exit codes.
  - Fixtures include at least one multipart case and one cleanup case.
  - Fixtures include a history append/rotation case.
- **Validation**:
  - `rg -n "^# api-rest fixtures" crates/api-rest/README.md`
  - `rg -n "multipart|cleanup|history" crates/api-rest/README.md`

### Task 1.4: Write api-gql parity spec
- **Location**:
  - `crates/api-gql/README.md`
- **Description**: Specify `api-gql` CLI behavior based on `gql.sh` and its helper scripts: arguments, env vars,
  operation and variables handling, JWT selection and login fallback, list commands, schema resolution, and exit codes.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Spec includes CLI surface (flags, env vars, list options) and exit code contract.
  - Spec captures variables min-limit normalization and login fallback rules.
  - Spec captures mutation detection semantics and how it influences safety in the suite runner.
  - Spec includes an explicit external-dependency inventory for api-gql (and the chosen policy per dependency).
- **Validation**:
  - `rg -n "^# api-gql parity spec" crates/api-gql/README.md`
  - `rg -n "GQL_VARS_MIN_LIMIT|login|schema" crates/api-gql/README.md`
  - `rg -n "External dependencies" crates/api-gql/README.md`

### Task 1.5: Write api-gql fixtures
- **Location**:
  - `crates/api-gql/README.md`
- **Description**: Define deterministic fixture scenarios for `api-gql`, including query success, errors present,
  allow-empty report gating, mutation detection, schema resolution behavior, and history behavior.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Fixtures cover both query and mutation-shaped operations.
  - Fixtures cover allow-empty report behavior and default non-empty requirements.
  - Fixtures cover list envs/jwts and schema resolution.
- **Validation**:
  - `rg -n "^# api-gql fixtures" crates/api-gql/README.md`
  - `rg -n "mutation|allow-empty|schema|history" crates/api-gql/README.md`

### Task 1.6: Write api-test parity spec
- **Location**:
  - `crates/api-test/README.md`
- **Description**: Specify `api-test` suite-runner behavior based on `api-test.sh` and `api-test-summary.sh`: suite file
  resolution, manifest schema v1, filtering, allow-writes guardrails, per-case artifacts, results JSON schema,
  JUnit output, and exit codes.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Spec defines suite manifest schema v1 and validation errors.
  - Spec defines selection semantics for --tag (AND), --only, --skip, and fail-fast.
  - Spec defines write guardrails and cleanup behavior gating.
  - Spec defines results JSON shape and exit code mapping (all pass vs failures vs invalid input).
  - Spec includes an explicit external-dependency inventory for api-test (and the chosen policy per dependency).
- **Validation**:
  - `rg -n "^# api-test parity spec" crates/api-test/README.md`
  - `rg -n "Suite schema v1|results|JUnit|Exit codes" crates/api-test/README.md`
  - `rg -n "External dependencies" crates/api-test/README.md`

### Task 1.7: Write api-test fixtures
- **Location**:
  - `crates/api-test/README.md`
- **Description**: Define deterministic suite-runner fixture scenarios: mixed REST + GraphQL suites, tagging filters,
  skip/only filters, allow-writes gating, rest-flow token extraction, auth JSON login caching, and cleanup steps.
- **Dependencies**:
  - Task 1.6
- **Complexity**: 5
- **Acceptance criteria**:
  - Fixtures cover pass, fail, skip outcomes and the corresponding exit codes.
  - Fixtures include at least one rest-flow case and one auth JSON driven suite.
  - Fixtures include at least one cleanup scenario for REST and for GraphQL.
- **Validation**:
  - `rg -n "^# api-test fixtures" crates/api-test/README.md`
  - `rg -n "rest-flow|auth|cleanup|tag|skip" crates/api-test/README.md`

## Sprint 2: Workspace scaffold (crates + CLI surfaces + smoke tests)
**Goal**: Create crates and CLI parsing surfaces for the three binaries plus a shared core crate.
**Demo/Validation**:
- Command(s): `cargo metadata --no-deps | rg "api-rest|api-gql|api-test|api-testing-core"`, `cargo run -p api-rest -- --help`
- Verify: help output exists and matches the spec intent; crates build in the workspace.

### Task 2.1: Add shared core crate
- **Location**:
  - `Cargo.toml`
  - `crates/api-testing-core/Cargo.toml`
  - `crates/api-testing-core/src/lib.rs`
- **Description**: Create a new library crate `api-testing-core` for shared logic (config discovery, history, JWT checks,
  jq-like JSON querying, redaction, report formatting, HTTP execution helpers).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace builds with the new crate present.
  - Core crate exposes modules with stable public APIs for the binaries to call.
- **Validation**:
  - `cargo check -p api-testing-core`
  - `cargo metadata --no-deps | rg "\"name\": \"api-testing-core\""`

### Task 2.2: Add api-rest binary crate with clap parsing
- **Location**:
  - `Cargo.toml`
  - `crates/api-rest/Cargo.toml`
  - `crates/api-rest/src/main.rs`
- **Description**: Create `api-rest` binary crate and implement clap parsing for the parity CLI surface:
  call (default), history, and report subcommands with flags consistent with the spec.
- **Dependencies**:
  - Task 2.1
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - `api-rest --help` and `api-rest report --help` exist and document the expected flags.
  - Unknown flags exit non-zero and print a clear error to stderr.
- **Validation**:
  - `cargo run -p api-rest -- --help`
  - `cargo run -p api-rest -- report --help`

### Task 2.3: Add api-gql binary crate with clap parsing
- **Location**:
  - `Cargo.toml`
  - `crates/api-gql/Cargo.toml`
  - `crates/api-gql/src/main.rs`
- **Description**: Create `api-gql` binary crate and implement clap parsing for the parity CLI surface:
  call (default), history, report, and schema subcommands (including list envs/jwts behavior).
- **Dependencies**:
  - Task 2.1
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - `api-gql --help` and `api-gql schema --help` exist and document the expected flags.
  - List commands are wired as separate flags or subcommands (per the spec) and exit 0 when invoked.
- **Validation**:
  - `cargo run -p api-gql -- --help`
  - `cargo run -p api-gql -- schema --help`

### Task 2.4: Add api-test binary crate with clap parsing
- **Location**:
  - `Cargo.toml`
  - `crates/api-test/Cargo.toml`
  - `crates/api-test/src/main.rs`
- **Description**: Create `api-test` binary crate and implement clap parsing for suite selection and outputs:
  `--suite`, `--suite-file`, `--tag`, `--only`, `--skip`, `--allow-writes`, `--out`, `--junit`, and `--fail-fast`,
  plus a `summary` mode that consumes results JSON.
- **Dependencies**:
  - Task 2.1
  - Task 1.6
- **Complexity**: 6
- **Acceptance criteria**:
  - `api-test --help` documents flags and expected exit codes.
  - Mutual exclusion of `--suite` and `--suite-file` is enforced.
  - Invalid flags exit non-zero with a clear error.
- **Validation**:
  - `cargo run -p api-test -- --help`
  - `cargo run -p api-test -- --suite smoke-demo --help`

### Task 2.5: Add CLI smoke tests for parsing and help output
- **Location**:
  - `crates/api-rest/tests/cli_smoke.rs`
  - `crates/api-gql/tests/cli_smoke.rs`
  - `crates/api-test/tests/cli_smoke.rs`
- **Description**: Add minimal integration tests for help output and argument validation to lock down the CLI surface early.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
  - Task 2.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests verify `--help` exits 0 and includes key option names.
  - Tests verify invalid flag parsing exits non-zero.
- **Validation**:
  - `cargo test -p api-rest --test cli_smoke`
  - `cargo test -p api-gql --test cli_smoke`
  - `cargo test -p api-test --test cli_smoke`

## Sprint 3: Shared core primitives (config, history, JWT, jq-engine, redaction)
**Goal**: Implement the shared primitives once, with unit tests, so the three binaries can stay small.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core`
- Verify: core modules cover the parsing and transformation rules from the specs.

### Task 3.1: Implement setup dir discovery and env file parsing
- **Location**:
  - `crates/api-testing-core/src/config.rs`
  - `crates/api-testing-core/src/env_file.rs`
- **Description**: Port the setup-dir discovery semantics for REST and GraphQL: upward search for endpoints/jwts/tokens/schema
  config files, plus default fallbacks (setup/rest and setup/graphql) when present. Implement an env-file parser compatible
  with the scripts (comments, optional export prefix, simple quoting rules).
- **Dependencies**:
  - Task 2.1
  - Task 1.2
  - Task 1.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Discovery is deterministic given a starting directory and optional explicit config dir.
  - Env parsing handles export prefix and quoted values the same way as the scripts.
  - Errors include actionable hints (for example, try --config-dir).
- **Validation**:
  - `cargo test -p api-testing-core config`

### Task 3.2: Implement JWT format and exp/nbf validation
- **Location**:
  - `crates/api-testing-core/src/jwt.rs`
- **Description**: Implement the script-equivalent JWT validation behavior: validate token structure, decode payload,
  and enforce exp/nbf (with leeway) when enabled, without doing signature verification.
- **Dependencies**:
  - Task 2.1
  - Task 1.2
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Matches strict vs warn behavior and error messaging intent from the scripts.
  - Leeway handling is covered by unit tests.
- **Validation**:
  - `cargo test -p api-testing-core jwt`

### Task 3.3: Implement jq-like JSON querying engine
- **Location**:
  - `crates/api-testing-core/src/jq.rs`
- **Description**: Provide a stable JSON query interface for the rest/gql runner and suite runner:
  boolean assertions, raw string extraction, and JSON transformations needed for redaction and templating.
  Prefer an embedded Rust jq-compatible engine; optionally allow a subprocess fallback when enabled for compatibility.
- **Dependencies**:
  - Task 2.1
  - Task 1.2
  - Task 1.6
- **Complexity**: 8
- **Acceptance criteria**:
  - Supports the jq usage patterns in the existing scripts (expect assertions, token extraction, credentials extraction,
    cleanup vars extraction, and redaction).
  - Query failures produce clear errors that include the expression and the context (file or response).
  - Unit tests cover both success and failure cases.
- **Validation**:
  - `cargo test -p api-testing-core jq`

### Task 3.4: Implement history append + rotation with locking
- **Location**:
  - `crates/api-testing-core/src/history.rs`
- **Description**: Port history behavior for REST and GraphQL: per-setup-dir history file, optional disabling, safe append
  under concurrency, optional URL masking, and size-based rotation with a configured keep count.
- **Dependencies**:
  - Task 2.1
  - Task 1.2
  - Task 1.4
- **Complexity**: 6
- **Acceptance criteria**:
  - History append is safe under concurrent processes (no corrupted file).
  - Rotation keeps the configured number of rotated files and honors disabled rotation.
  - Output matches the command-snippet shape from the scripts (token and jwt values must be redacted).
- **Validation**:
  - `cargo test -p api-testing-core history`

### Task 3.5: Implement redaction helpers and markdown rendering primitives
- **Location**:
  - `crates/api-testing-core/src/redact.rs`
  - `crates/api-testing-core/src/markdown.rs`
- **Description**: Implement shared helpers for report generation: deterministic JSON formatting, markdown blocks, and default
  redaction rules for common secret-bearing fields (tokens, passwords, cookies, authorization headers).
- **Dependencies**:
  - Task 2.1
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Redaction is applied by default, and can be disabled explicitly by a flag.
  - Markdown output is stable across platforms (line endings, ordering of JSON keys when applicable).
- **Validation**:
  - `cargo test -p api-testing-core redact`

## Sprint 4: Implement api-rest (call + history + report) with integration tests
**Goal**: Achieve parity for REST calls and reporting.
**Demo/Validation**:
- Command(s): `cargo test -p api-rest`, `cargo run -p api-rest -- call setup/rest/requests/health.request.json`
- Verify: output matches spec expectations and fixtures.

### Task 4.1: Implement REST request schema parsing and validation
- **Location**:
  - `crates/api-testing-core/src/rest/schema.rs`
  - `crates/api-rest/src/main.rs`
- **Description**: Implement the REST request JSON schema: validate method/path/query/headers/body/multipart, reject invalid
  combinations, and extract expect and cleanup definitions.
- **Dependencies**:
  - Task 3.3
  - Task 3.1
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Invalid request files produce deterministic, actionable errors.
  - Query encoding rules match the scripts (scalars or arrays of scalars only).
  - Multipart and body mutual exclusion is enforced.
- **Validation**:
  - `cargo test -p api-testing-core rest_schema`

### Task 4.2: Implement REST HTTP execution (including multipart)
- **Location**:
  - `crates/api-testing-core/src/rest/runner.rs`
- **Description**: Execute REST calls via a Rust HTTP client, supporting headers, query, JSON body, and multipart file uploads,
  and capturing status, headers (as needed), and body.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Multipart supports filePath and contentType and reads files relative to the request file by default.
  - Response body is written to stdout (JSON or raw), with stderr used for errors.
  - Non-JSON failure bodies can be surfaced in non-interactive contexts (per spec).
- **Validation**:
  - `cargo test -p api-testing-core rest_runner`

### Task 4.3: Implement REST expect and cleanup helpers
- **Location**:
  - `crates/api-testing-core/src/rest/expect.rs`
  - `crates/api-testing-core/src/rest/cleanup.rs`
- **Description**: Implement expect.status and expect.jq evaluation and the cleanup request generation and execution rules.
  Ensure cleanup templating and vars extraction is consistent with the current behavior.
- **Dependencies**:
  - Task 4.2
  - Task 3.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Expect failures exit non-zero and print a clear reason.
  - Cleanup path templating resolves to an absolute path and rejects invalid results.
  - Cleanup supports expectStatus and optional expect.jq.
- **Validation**:
  - `cargo test -p api-testing-core rest_expect`

### Task 4.4: Implement api-rest history and report commands
- **Location**:
  - `crates/api-rest/src/main.rs`
  - `crates/api-testing-core/src/rest/report.rs`
- **Description**: Implement `api-rest history` and `api-rest report` parity: command-only replay snippets, report file naming
  defaults under docs, redaction behavior, and flags for including command snippets and URL masking.
- **Dependencies**:
  - Task 4.2
  - Task 4.3
  - Task 3.4
  - Task 3.5
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Report includes endpoint selection and request file reference, and is secret-safe by default.
  - Report can run the request or accept a response file or stdin.
  - History output matches expected snippet shape and does not include secret token values.
- **Validation**:
  - `cargo test -p api-rest`

### Task 4.5: Add api-rest integration tests with a local HTTP server
- **Location**:
  - `crates/api-rest/tests/integration.rs`
- **Description**: Add deterministic integration tests for success paths and error paths using a local HTTP test server and
  temporary setup directories (endpoints, tokens, requests, and files for multipart).
- **Dependencies**:
  - Task 4.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover expect.status success and failure, plus expect.jq failure.
  - Tests cover multipart and cleanup behavior.
  - Tests do not require network access beyond localhost.
- **Validation**:
  - `cargo test -p api-rest --test integration`

## Sprint 5: Implement api-gql (call + history + report + schema) with integration tests
**Goal**: Achieve parity for GraphQL calls and reporting.
**Demo/Validation**:
- Command(s): `cargo test -p api-gql`, `cargo run -p api-gql -- call setup/graphql/operations/countries.graphql setup/graphql/operations/countries.variables.json`
- Verify: output matches spec expectations and fixtures.

### Task 5.1: Implement GraphQL operation and variables handling
- **Location**:
  - `crates/api-testing-core/src/graphql/schema.rs`
  - `crates/api-testing-core/src/graphql/vars.rs`
- **Description**: Implement operation file loading and optional variables loading, including variables min-limit bumping and
  canonical formatting for report generation.
- **Dependencies**:
  - Task 3.3
  - Task 1.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Variables JSON is validated and normalized deterministically when min-limit is enabled.
  - Errors include the offending file path and the reason.
- **Validation**:
  - `cargo test -p api-testing-core graphql_vars`

### Task 5.2: Implement GraphQL auth selection and login fallback
- **Location**:
  - `crates/api-testing-core/src/graphql/auth.rs`
- **Description**: Implement JWT profile selection (jwts.env and overrides) and the login fallback behavior when the selected
  profile is missing or empty, using a configured login operation under setup/graphql.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Matches the intended fallback search order for login operation files.
  - JWT validation is applied consistently when enabled.
  - Clear errors are produced when auth is required but cannot be resolved.
- **Validation**:
  - `cargo test -p api-testing-core graphql_auth`

### Task 5.3: Implement GraphQL request execution and response validation
- **Location**:
  - `crates/api-testing-core/src/graphql/runner.rs`
  - `crates/api-testing-core/src/graphql/expect.rs`
- **Description**: Execute GraphQL calls via a Rust HTTP client, print response JSON to stdout, and implement default and
  configured validations (errors present, allowErrors rules, expect.jq behavior in the suite runner).
- **Dependencies**:
  - Task 5.1
  - Task 5.2
  - Task 3.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Default behavior fails when errors are present and allowErrors is false.
  - When allowErrors is true, expect.jq is required (per spec) and is enforced.
  - Output contains only the response body JSON.
- **Validation**:
  - `cargo test -p api-testing-core graphql_runner`

### Task 5.4: Implement api-gql history, report, list, and schema commands
- **Location**:
  - `crates/api-gql/src/main.rs`
  - `crates/api-testing-core/src/graphql/report.rs`
  - `crates/api-testing-core/src/graphql/schema_file.rs`
- **Description**: Implement history writing, report file generation with redaction and allow-empty gating, list env/jwt
  output, and schema path resolution and cat behavior.
- **Dependencies**:
  - Task 3.4
  - Task 3.5
  - Task 5.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Report generation blocks empty reports by default and requires an explicit allow-empty flag.
  - Schema resolution matches the configured file and fallback candidates.
  - List commands print deterministic sorted output.
- **Validation**:
  - `cargo test -p api-gql`

### Task 5.5: Add api-gql integration tests with a local HTTP server
- **Location**:
  - `crates/api-gql/tests/integration.rs`
- **Description**: Add integration tests using a local HTTP test server and temporary setup directories (endpoints, jwts,
  schema, operations, variables). Cover auth fallback, vars min-limit, allow-empty behavior, and error handling.
- **Dependencies**:
  - Task 5.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover success with data, failure with errors, and allow-empty report gating.
  - Tests cover vars min-limit normalization (including nested limit fields).
- **Validation**:
  - `cargo test -p api-gql --test integration`

## Sprint 6: Implement api-test suite runner (run + summary) with end-to-end tests
**Goal**: Achieve parity for suite execution, results JSON, JUnit output, summary generation, and cleanup.
**Demo/Validation**:
- Command(s): `cargo test -p api-test`, `cargo run -p api-test -- --suite smoke-demo --out out/api-test-runner/results.json`
- Verify: results JSON matches spec and artifacts are written under out/api-test-runner.

### Task 6.1: Implement suite schema v1 parsing and validation
- **Location**:
  - `crates/api-testing-core/src/suite/schema.rs`
  - `crates/api-test/src/main.rs`
- **Description**: Implement serde parsing for suite schema v1 plus semantic validation: required fields, valid types,
  mutual exclusions, and actionable error messages (including the case id and JSON path when possible).
- **Dependencies**:
  - Task 5.3
  - Task 4.3
  - Task 1.6
- **Complexity**: 8
- **Acceptance criteria**:
  - Invalid suite files fail fast with exit code reserved for invalid inputs.
  - Errors identify the failing field and case id.
- **Validation**:
  - `cargo test -p api-testing-core suite_schema`

### Task 6.2: Implement suite resolution and selection filters
- **Location**:
  - `crates/api-testing-core/src/suite/resolve.rs`
  - `crates/api-testing-core/src/suite/filter.rs`
- **Description**: Implement suite file resolution for `--suite` and `--suite-file`, support overriding suites dir via env,
  and implement selection filters (tags AND semantics, only, skip, fail-fast).
- **Dependencies**:
  - Task 6.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Resolution matches the canonical search order.
  - Filters behave deterministically and are covered by unit tests.
- **Validation**:
  - `cargo test -p api-testing-core suite_filter`

### Task 6.3: Implement allow-writes guardrails and mutation detection
- **Location**:
  - `crates/api-testing-core/src/suite/safety.rs`
  - `crates/api-testing-core/src/graphql/mutation.rs`
- **Description**: Implement suite-level and case-level write gating, and GraphQL mutation detection used to classify cases
  as write-capable. Ensure write-capable cases are skipped unless writes are enabled and allowWrite is true.
- **Dependencies**:
  - Task 5.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Mutation detection matches the existing intent and is covered by unit tests.
  - Skip reasons are stable and informative for summaries.
- **Validation**:
  - `cargo test -p api-testing-core mutation`

### Task 6.4: Implement suite execution engine and per-case artifacts
- **Location**:
  - `crates/api-testing-core/src/suite/runner.rs`
  - `crates/api-testing-core/src/suite/results.rs`
- **Description**: Implement the suite execution loop that runs REST and GraphQL cases by calling the Rust runners,
  captures per-case stdout/stderr to files under an output directory, tracks durations, and builds a results JSON object.
- **Dependencies**:
  - Task 6.2
  - Task 4.2
  - Task 5.3
- **Complexity**: 9
- **Acceptance criteria**:
  - Results JSON always prints to stdout.
  - Output directory structure matches spec and includes per-case stdout/stderr paths.
  - Exit codes match: all pass vs failures vs invalid input.
- **Validation**:
  - `cargo test -p api-testing-core suite_runner`

### Task 6.5: Implement auth JSON login and token caching
- **Location**:
  - `crates/api-testing-core/src/suite/auth.rs`
- **Description**: Implement suite auth blocks: read secret JSON from the configured env var, extract per-profile credentials,
  perform login via REST or GraphQL, cache tokens per profile, and inject tokens into subsequent cases safely.
- **Dependencies**:
  - Task 6.4
  - Task 3.3
- **Complexity**: 9
- **Acceptance criteria**:
  - Credential extraction and token extraction use the shared jq engine and are covered by unit tests.
  - Tokens are never printed in command snippets or logs by default.
  - Auth failures are surfaced with actionable hints.
- **Validation**:
  - `cargo test -p api-testing-core suite_auth`

### Task 6.6: Implement cleanup steps for REST and GraphQL cases
- **Location**:
  - `crates/api-testing-core/src/suite/cleanup.rs`
- **Description**: Implement cleanup execution after a write-capable case: REST cleanup requests and GraphQL cleanup ops,
  templating from response values, optional per-step expects, and robust logging when cleanup fails.
- **Dependencies**:
  - Task 6.4
  - Task 6.5
  - Task 3.3
- **Complexity**: 9
- **Acceptance criteria**:
  - Cleanup runs only when writes are enabled (or when configured by spec) and main response exists.
  - Cleanup failures are recorded but do not corrupt the main results JSON.
  - Cleanup supports both single-step and multi-step definitions.
- **Validation**:
  - `cargo test -p api-testing-core suite_cleanup`

### Task 6.7: Implement JUnit output and Markdown summary generation
- **Location**:
  - `crates/api-testing-core/src/suite/junit.rs`
  - `crates/api-testing-core/src/suite/summary.rs`
  - `crates/api-test/src/main.rs`
- **Description**: Implement optional JUnit XML emission and a Markdown summary generator that consumes the results JSON and
  optionally appends to the GitHub step summary file when configured by environment.
- **Dependencies**:
  - Task 6.4
- **Complexity**: 7
- **Acceptance criteria**:
  - JUnit output is valid XML and includes per-case timing and pass/fail status.
  - Summary output is deterministic and truncates large lists according to flags.
  - Summary supports reading from a file or stdin.
- **Validation**:
  - `cargo test -p api-testing-core junit`
  - `cargo test -p api-testing-core summary`

### Task 6.8: Add api-test end-to-end integration tests (REST + GraphQL)
- **Location**:
  - `crates/api-test/tests/e2e.rs`
- **Description**: Add end-to-end tests that build a temporary repo-like directory layout, start a local HTTP server that can
  respond to both REST and GraphQL requests, run a small suite, and validate the emitted results JSON, JUnit file, and
  artifacts layout.
- **Dependencies**:
  - Task 6.7
- **Complexity**: 9
- **Acceptance criteria**:
  - Tests cover pass, fail, and skip cases (including write gating).
  - Tests validate that secrets are not present in stdout/stderr artifacts.
  - Tests validate JUnit and summary outputs exist when enabled.
- **Validation**:
  - `cargo test -p api-test --test e2e`

## Sprint 7: Polish, docs, and pre-delivery checks
**Goal**: Make the binaries easy to adopt and ensure workspace checks pass.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: fmt, clippy, tests, and zsh completion tests are green.

### Task 7.1: Add user-facing usage docs and migration notes
- **Location**:
  - `crates/api-testing-core/README.md`
- **Description**: Document how to migrate from calling the legacy scripts to using the new binaries, including directory
  layout expectations, CI usage patterns, and how to keep report outputs consistent with existing contracts.
- **Dependencies**:
  - Task 4.4
  - Task 5.4
  - Task 6.7
- **Complexity**: 4
- **Acceptance criteria**:
  - Docs include concrete command examples for REST, GraphQL, and suite runs.
  - Docs include guidance for report generation and summary usage.
- **Validation**:
  - `rg -n "^# " crates/api-testing-core/README.md`
  - `rg -n "api-rest|api-gql|api-test" crates/api-testing-core/README.md`

### Task 7.2: Run mandatory repo checks and fix in-scope failures
- **Location**:
  - `DEVELOPMENT.md`
- **Description**: Run the repo’s mandatory checks (fmt, clippy, cargo test workspace, zsh completion tests) and address any
  failures that are caused by this port work.
- **Dependencies**:
  - Task 7.1
- **Complexity**: 5
- **Acceptance criteria**:
  - All mandatory checks pass locally.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: validate parsing, env resolution, JWT validation, jq engine, templating, and redaction in `api-testing-core`.
- Integration: local HTTP test server exercises `api-rest` and `api-gql` end-to-end with temp setup directories.
- E2E: `api-test` executes a mixed suite (REST + GraphQL) against the same local server, validates JSON + JUnit + summary outputs.

## Risks & gotchas
- jq-compatibility: embedded jq engines may not support every jq feature; mitigate via a compatibility matrix in docs and
  an optional subprocess fallback when enabled.
- Multipart correctness: parity around file paths, content types, and boundary encoding must be tested carefully.
- Secret leakage: multiple output surfaces (stdout, stderr, history, report, artifacts) require consistent redaction rules.
- GraphQL error semantics: some servers return HTTP 200 with errors; validation rules must be explicit and well-tested.
- Cleanup safety: cleanup steps should be robust, best-effort, and never corrupt the main results contract.

## Rollback plan
- Keep using the existing scripts; the Rust binaries are additive and can be removed from PATH without breaking existing repos.
- If necessary, revert the workspace changes that add the new crates and docs, leaving unrelated crates untouched.
