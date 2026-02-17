# Plan: Agent required-doc discovery CLI

## Overview
Build a new workspace CLI (`agent-docs`) that helps Codex/agents resolve the mandatory markdown documents they must follow at startup and during task execution. The CLI does not replace `AGENTS.md`; it provides a deterministic lookup layer for both `AGENTS_HOME` and project-level policy files, including override precedence (`AGENTS.override.md` over `AGENTS.md`). The design includes a TOML extension mechanism so teams can register additional required docs (for example `BINARY_DEPENDENCIES.md` in `nils-cli`) without hard-coding new rules. The CLI also provides a default `AGENTS.md` template scaffold for new repos so agents are explicitly instructed to use `agent-docs` and `AGENT_DOCS.toml`, plus a baseline-audit/remediation flow for missing minimum docs.

## Scope
- In scope: new Rust CLI crate, context-based document resolution, built-in rule set for home/project policies, TOML-based extension files, optional write commands to append/update TOML entries, default `AGENTS.md` template scaffold for bootstrap, baseline missing-doc detection, and scaffold commands to generate missing baseline docs, tests, docs, wrappers/completions.
- Out of scope: replacing AGENTS loading in agent runtimes, in-place auto-editing of existing custom `AGENTS.md` files, auto-discovering non-markdown policies from arbitrary tools, remote config sync.

## Assumptions (if any)
1. Binary/crate name is `agent-docs` and it will be a new workspace member under `crates/agent-docs`.
2. Default extension files are `AGENT_DOCS.toml` at both scopes: `$AGENTS_HOME/AGENT_DOCS.toml` and `$PROJECT_PATH/AGENT_DOCS.toml`.
3. Project root resolution order is: `PROJECT_PATH` env var -> `git rev-parse --show-toplevel` from cwd -> current working directory.
4. The CLI remains read-only unless users run an explicit mutation command (`agent-docs add ...`) targeting `home` or `project` TOML.

## Compatibility contract (required behavior)
- Contexts:
  - `startup`: always include global and project AGENTS policy (`AGENTS.override.md` preferred over `AGENTS.md` per scope).
  - `skill-dev`: include `DEVELOPMENT.md` in `AGENTS_HOME` when skill development/management is in scope.
  - `task-tools`: include `CLI_TOOLS.md` in `AGENTS_HOME` when task requires tool selection guidance.
  - `project-dev`: include project `DEVELOPMENT.md` for development/modification tasks.
- Output:
  - `resolve` returns an ordered, de-duplicated list with reason metadata (`why`, `scope`, `required`, `source`).
  - Missing optional docs are reported as warnings in text mode and structured flags in JSON mode.
  - Missing required docs return non-zero only when `--strict` is enabled.
- Extension:
  - TOML entries can add required/optional docs per context and scope.
  - Built-in docs remain immutable defaults and cannot be removed by TOML.
- Template scaffold:
  - `scaffold-agents` creates a default `AGENTS.md` only when missing (or when `--force` is explicit).
  - The generated template explicitly instructs agents to run `agent-docs resolve` for startup and task contexts and to consult `AGENT_DOCS.toml` for project-specific additions.
- Missing baseline workflow:
  - `baseline --check` reports minimum-baseline docs by scope with `present`/`missing` status and machine-readable JSON output.
  - When required baseline docs are missing, output must include actionable create suggestions (for example `scaffold-baseline --missing-only --target project`).
  - Caller agents must first prompt the user before creating missing docs.
  - If user approves and provides no extra writing instructions, `scaffold-baseline` generates baseline docs from repository signals (for example `README.md`, `Cargo.toml`, existing scripts, and current directory structure).

## Minimum Baseline 文件清單
- `AGENTS_HOME`:
  - Required for startup: `AGENTS.override.md` or `AGENTS.md`
  - Required for skill development tasks: `DEVELOPMENT.md`
  - Required for tool-selection tasks: `CLI_TOOLS.md`
- `PROJECT_PATH`:
  - Required for startup: `AGENTS.override.md` or `AGENTS.md`
  - Required for project development/modification tasks: `DEVELOPMENT.md`
- Extension baseline:
  - `AGENT_DOCS.toml` is optional, but any document marked `required=true` becomes part of effective baseline checks for that context/scope.

## Sprint 1: Contract + fixture-first design
**Goal**: lock the CLI contract and fixture matrix before implementation so behavior is testable and stable.
**Demo/Validation**:
- Command(s): `rg -n "Compatibility contract|AGENT_DOCS.toml|context" crates/agent-docs/README.md`
- Verify: README clearly defines contexts, precedence, output schema, and TOML extension semantics.

### Task 1.1: Write the product spec for `agent-docs`
- **Location**:
  - `crates/agent-docs/README.md`
- **Description**: Create a full behavior spec: command list, flags, exit codes, context mapping, scope precedence (`override` vs default), strict-mode semantics, and non-goals (especially “not replacing AGENTS runtime loading”).
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - README defines all supported contexts and required built-in docs per context.
  - README documents path resolution precedence for `AGENTS_HOME` and `PROJECT_PATH`.
  - README includes output examples for text and JSON mode.
- **Validation**:
  - `test -f crates/agent-docs/README.md`
  - `rg -n "startup|skill-dev|task-tools|project-dev" crates/agent-docs/README.md`

### Task 1.2: Define TOML schema and merge contract
- **Location**:
  - `crates/agent-docs/README.md`
  - `docs/plans/agent-doc-discovery-cli-plan.md`
- **Description**: Define `AGENT_DOCS.toml` schema with explicit fields (`context`, `scope`, `path`, `required`, `when`, `notes`) and deterministic merge rules across home/project scopes.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Schema supports adding `BINARY_DEPENDENCIES.md` for `project-dev` context.
  - Merge order is explicit and deterministic for duplicate paths.
  - Invalid schema behavior and error messages are documented.
- **Validation**:
  - `rg -n "AGENT_DOCS\.toml|BINARY_DEPENDENCIES\.md|merge" crates/agent-docs/README.md`

### Task 1.3: Build fixture matrix for scope/context combinations
- **Location**:
  - `crates/agent-docs/tests/fixtures/home/AGENTS.md`
  - `crates/agent-docs/tests/fixtures/home/AGENTS.override.md`
  - `crates/agent-docs/tests/fixtures/project/DEVELOPMENT.md`
  - `crates/agent-docs/tests/fixtures/config/AGENT_DOCS.valid.toml`
  - `crates/agent-docs/tests/fixtures/config/AGENT_DOCS.invalid.toml`
- **Description**: Create fixture trees that cover `AGENTS.md` + `AGENTS.override.md`, missing-file cases, and TOML extensions (including `BINARY_DEPENDENCIES.md`) for deterministic tests.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Fixtures include at least one case for each context.
  - Fixtures include both valid and invalid `AGENT_DOCS.toml` examples.
  - Fixtures include duplicate path scenarios to verify de-dup behavior.
- **Validation**:
  - `find crates/agent-docs/tests/fixtures -type f | rg "AGENTS|DEVELOPMENT|CLI_TOOLS|BINARY_DEPENDENCIES|AGENT_DOCS"`

**Parallelization**:
- Task 1.3 can run in parallel after Task 1.2 schema fields are frozen.

## Sprint 2: Core CLI + built-in resolver + baseline audit
**Goal**: deliver usable `resolve` behavior for built-in rules, deterministic environment/path resolution, and baseline missing-doc diagnostics.
**Demo/Validation**:
- Command(s): `cargo run -q -p agent-docs -- resolve --context startup --format json`
- Verify: output includes both home and project AGENTS policy paths with precedence metadata.

### Task 2.1: Scaffold crate and CLI surface
- **Location**:
  - `Cargo.toml`
  - `crates/agent-docs/Cargo.toml`
  - `crates/agent-docs/src/main.rs`
  - `crates/agent-docs/src/cli.rs`
  - `crates/agent-docs/src/lib.rs`
- **Description**: Add `agent-docs` crate and command surface (`resolve`, `contexts`, `add`, `scaffold-agents`, `baseline`, `scaffold-baseline`) with clap argument parsing, help output, and usage exit codes.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `cargo run -q -p agent-docs -- --help` succeeds.
  - `resolve --context startup` and `contexts` parse correctly.
  - `scaffold-agents` argument parsing works for `--target`, `--force`, and optional output path.
  - `baseline --check` and `scaffold-baseline` argument parsing work for `--target`, `--missing-only`, and `--force`.
  - Unknown command/invalid arguments return usage exit code.
- **Validation**:
  - `cargo run -q -p agent-docs -- --help`
  - `cargo run -q -p agent-docs -- contexts`
  - `cargo run -q -p agent-docs -- baseline --check --target project`

### Task 2.2: Implement environment and root resolution
- **Location**:
  - `crates/agent-docs/src/env.rs`
  - `crates/agent-docs/src/paths.rs`
- **Description**: Implement `AGENTS_HOME` and `PROJECT_PATH` resolution contract with fallbacks, path normalization, and scope root discovery used by resolver and tests.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - `PROJECT_PATH` override is honored when provided.
  - Without `PROJECT_PATH`, Git top-level is used if available.
  - If Git top-level is unavailable, cwd fallback works without panic.
- **Validation**:
  - `cargo test -p agent-docs --tests`

### Task 2.3: Implement built-in mandatory-doc resolver
- **Location**:
  - `crates/agent-docs/src/model.rs`
  - `crates/agent-docs/src/resolver.rs`
  - `crates/agent-docs/src/output.rs`
- **Description**: Implement context mapping and precedence for built-ins: `AGENTS.override.md|AGENTS.md`, `DEVELOPMENT.md`, and `CLI_TOOLS.md`, returning ordered entries with required flags and source reasons.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - `startup` returns AGENTS documents from both scopes with override-first preference.
  - `skill-dev` and `task-tools` include AGENTS_HOME docs when present.
  - `project-dev` includes project `DEVELOPMENT.md` rule.
  - `--strict` exits non-zero when required docs are missing.
- **Validation**:
  - `cargo run -q -p agent-docs -- resolve --context startup --strict`
  - `cargo run -q -p agent-docs -- resolve --context project-dev --format json | python3 -m json.tool`

### Task 2.4: Add resolver integration tests for built-in rules
- **Location**:
  - `crates/agent-docs/tests/common.rs`
  - `crates/agent-docs/tests/resolve_builtin.rs`
- **Description**: Add integration tests against fixture trees for context behavior, precedence, strict mode, and missing-doc diagnostics.
- **Dependencies**:
  - Task 1.3
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover all built-in contexts and both output formats.
  - Tests assert deterministic ordering and de-dup behavior.
  - Tests assert strict/non-strict exit code differences.
- **Validation**:
  - `cargo test -p agent-docs resolve_builtin`

### Task 2.5: Implement baseline check command for minimum docs
- **Location**:
  - `crates/agent-docs/src/commands/baseline.rs`
  - `crates/agent-docs/src/output.rs`
  - `crates/agent-docs/tests/baseline.rs`
- **Description**: Implement `baseline --check` to evaluate the Minimum Baseline list for selected scope (`home`, `project`, or `all`), report missing docs, and emit suggested remediation commands.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - `baseline --check --target project` reports missing/present status for project baseline docs.
  - JSON output includes `missing_required`, `missing_optional`, and `suggested_actions`.
  - `--strict` returns non-zero when required baseline docs are missing.
- **Validation**:
  - `cargo test -p agent-docs baseline`

**Parallelization**:
- Task 2.2 can proceed in parallel with CLI parser refinement in Task 2.1 after initial crate scaffold.

## Sprint 3: TOML extension, AGENTS template scaffold, baseline auto-generation, and registration workflow
**Goal**: allow teams/skills to add context-aware required docs via TOML, bootstrap new repositories with a default `AGENTS.md`, and auto-generate missing baseline docs after user approval.
**Demo/Validation**:
- Command(s): `cargo run -q -p agent-docs -- add --target project --context project-dev --path BINARY_DEPENDENCIES.md --required`
- Verify: project `AGENT_DOCS.toml` is created/updated and `resolve --context project-dev` includes the new doc.

### Task 3.1: Implement TOML config parsing and validation
- **Location**:
  - `crates/agent-docs/src/config.rs`
  - `crates/agent-docs/src/model.rs`
- **Description**: Parse and validate `AGENT_DOCS.toml` from home/project scopes, enforce schema constraints, and emit actionable validation errors for malformed entries.
- **Dependencies**:
  - Task 1.2
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Valid TOML files load from both scopes.
  - Invalid entries fail with explicit line/context messages.
  - Unsupported context names are rejected.
- **Validation**:
  - `cargo test -p agent-docs --tests`

### Task 3.2: Merge built-ins with TOML entries deterministically
- **Location**:
  - `crates/agent-docs/src/resolver.rs`
- **Description**: Merge TOML entries with built-in defaults by context/scope, preserving built-in mandatory docs and enforcing deterministic order + de-duplication.
- **Dependencies**:
  - Task 2.3
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - TOML can add `BINARY_DEPENDENCIES.md` to `project-dev` context.
  - Duplicate entries from home/project configs are deduplicated with stable precedence.
  - Built-in required docs remain present even when TOML omits them.
- **Validation**:
  - `cargo test -p agent-docs resolve_toml`

### Task 3.3: Implement `add` command for TOML upsert
- **Location**:
  - `crates/agent-docs/src/commands/add.rs`
  - `crates/agent-docs/src/main.rs`
- **Description**: Implement `add` command to create/update `AGENT_DOCS.toml` entries for `home` or `project` scope with idempotent upsert behavior and stable file formatting.
- **Dependencies**:
  - Task 2.1
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `add` creates missing target TOML and appends a valid entry.
  - Re-running the same command updates existing matching entry instead of duplication.
  - `--required` and context/scope flags are persisted accurately.
- **Validation**:
  - `cargo test -p agent-docs --tests`

### Task 3.4: Add end-to-end tests for extension workflow
- **Location**:
  - `crates/agent-docs/tests/resolve_toml.rs`
  - `crates/agent-docs/tests/add.rs`
- **Description**: Add E2E tests covering add->resolve flow, malformed TOML recovery messages, and `BINARY_DEPENDENCIES.md` discoverability in `project-dev` context.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests verify full add->resolve workflow for both scopes.
  - Tests verify malformed TOML produces non-zero with actionable stderr.
  - Tests verify `BINARY_DEPENDENCIES.md` can be introduced through TOML and resolved in output.
- **Validation**:
  - `cargo test -p agent-docs resolve_toml add`

### Task 3.5: Implement default `AGENTS.md` template scaffold
- **Location**:
  - `crates/agent-docs/src/commands/scaffold_agents.rs`
  - `crates/agent-docs/src/templates/agents_default.md`
  - `crates/agent-docs/src/main.rs`
- **Description**: Implement `scaffold-agents` command to create a default `AGENTS.md` in `project` or `home` scope. The template must explicitly direct agents to use `agent-docs resolve --context startup` and `agent-docs resolve --context project-dev` during startup and development tasks, and mention `AGENT_DOCS.toml` as the extension point.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `scaffold-agents --target project` creates `$PROJECT_PATH/AGENTS.md` when missing.
  - Generated content includes concrete `agent-docs resolve` examples for `startup` and `project-dev`.
  - Generated content mentions `AGENT_DOCS.toml` for additional required docs.
  - If target file already exists, command exits non-zero unless `--force` is set.
- **Validation**:
  - `cargo test -p agent-docs scaffold_agents`

### Task 3.6: Add integration tests for template creation and overwrite safety
- **Location**:
  - `crates/agent-docs/tests/scaffold_agents.rs`
  - `crates/agent-docs/tests/fixtures/project/AGENTS.template.expected.md`
- **Description**: Add integration tests for first-time scaffold, no-overwrite safety, force overwrite behavior, and template content assertions for `agent-docs`/`AGENT_DOCS.toml` guidance.
- **Dependencies**:
  - Task 3.5
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests assert non-force execution does not overwrite existing `AGENTS.md`.
  - Tests assert `--force` overwrites and matches expected template fixture.
  - Tests assert required guidance strings are present in scaffolded output.
- **Validation**:
  - `cargo test -p agent-docs scaffold_agents`

### Task 3.7: Implement `scaffold-baseline` for missing baseline docs
- **Location**:
  - `crates/agent-docs/src/commands/scaffold_baseline.rs`
  - `crates/agent-docs/src/templates/development_default.md`
  - `crates/agent-docs/src/templates/cli_tools_default.md`
  - `crates/agent-docs/src/main.rs`
- **Description**: Implement `scaffold-baseline` command to create missing baseline docs (`AGENTS.md`, `DEVELOPMENT.md`, `CLI_TOOLS.md`) for target scope. When no additional writing instructions are provided, generate defaults using repository signals (detected language/toolchain, existing scripts, and command conventions).
- **Dependencies**:
  - Task 2.5
  - Task 3.1
  - Task 3.5
- **Complexity**: 8
- **Acceptance criteria**:
  - `scaffold-baseline --target project --missing-only` creates only missing project baseline docs.
  - Generated `DEVELOPMENT.md` includes runnable setup/build/test commands aligned with detected project conventions.
  - Existing files are not overwritten unless `--force` is provided.
  - Command output clearly states created/skipped files.
- **Validation**:
  - `cargo test -p agent-docs scaffold_baseline`

### Task 3.8: Add integration tests for baseline auto-generation flow
- **Location**:
  - `crates/agent-docs/tests/scaffold_baseline.rs`
  - `crates/agent-docs/tests/fixtures/project/DEVELOPMENT.template.expected.md`
  - `crates/agent-docs/tests/fixtures/home/CLI_TOOLS.template.expected.md`
- **Description**: Add tests for missing-doc generation, missing-only behavior, forced overwrite, and content contracts for generated baseline docs.
- **Dependencies**:
  - Task 3.7
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests verify only missing files are created in missing-only mode.
  - Tests verify forced overwrite updates existing baseline files.
  - Tests verify generated templates contain required sections and actionable commands.
- **Validation**:
  - `cargo test -p agent-docs scaffold_baseline`

**Parallelization**:
- Task 3.2 and Task 3.3 can run in parallel after Task 3.1 stabilizes the schema model.
- Task 3.5 can run in parallel with Task 3.4 after Task 3.1 and Task 2.2 are complete.
- Task 3.7 can run in parallel with Task 3.6 after Task 2.5 and Task 3.5 are complete.

## Sprint 4: Workspace integration, completions, and delivery gates
**Goal**: make the new CLI consumable by humans, skills, and wrappers with full repo checks passing.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify: workspace fmt/clippy/tests/zsh completion checks pass with new CLI integrated.

### Task 4.1: Add shell completion and wrapper integration
- **Location**:
  - `completions/zsh/_agent-docs`
  - `completions/bash/agent-docs`
  - `wrappers/agent-docs`
- **Description**: Add completion files and wrapper script for `agent-docs`, matching repo conventions for command naming and completion loading.
- **Dependencies**:
  - Task 2.1
  - Task 3.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Zsh and Bash completion files provide subcommand and flag completion.
  - Wrapper executes installed binary consistently with existing wrappers.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.2: Update documentation and dependency references
- **Location**:
  - `README.md`
  - `BINARY_DEPENDENCIES.md`
  - `crates/agent-docs/README.md`
- **Description**: Document CLI purpose, contexts, env vars, TOML extension usage, and mention where/when `BINARY_DEPENDENCIES.md` should be registered for project development contexts.
- **Dependencies**:
  - Task 3.4
  - Task 4.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Root README includes `agent-docs` in workspace layout and quick usage.
  - `BINARY_DEPENDENCIES.md` references `agent-docs add` usage for extension registration.
  - Crate README has copy-pastable examples for `resolve` and `add`.
- **Validation**:
  - `rg -n "agent-docs" README.md BINARY_DEPENDENCIES.md crates/agent-docs/README.md`

### Task 4.3: Run mandatory checks and coverage threshold
- **Location**:
  - `DEVELOPMENT.md`
  - `crates/agent-docs/tests/resolve_builtin.rs`
  - `crates/agent-docs/tests/resolve_toml.rs`
- **Description**: Run repo-required checks and ensure new crate test coverage does not reduce workspace coverage below policy.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - `cargo fmt --all -- --check` passes.
  - `cargo clippy --all-targets --all-features -- -D warnings` passes.
  - `cargo test --workspace` passes.
  - `zsh -f tests/zsh/completion.test.zsh` passes.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - Optional: `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80` (when `cargo-llvm-cov` and `cargo-nextest` are installed)

**Parallelization**:
- Task 4.1 and Task 4.2 can run in parallel once core CLI behavior (Sprint 3) is stable.

## Testing Strategy
- Unit:
  - Parser/validator tests for context enums, TOML schema validation, and path resolution edge cases.
- Integration:
  - Fixture-driven command tests for `resolve`, `contexts`, and `add` across home/project scope combinations.
- E2E/manual:
  - Use a temp `AGENTS_HOME` plus temp project repo, run `agent-docs resolve` in each context, and confirm emitted docs match expected policy files.

## Risks & gotchas
- Ambiguous policy precedence can cause incorrect agent behavior; mitigation is explicit order metadata in output and strict fixture assertions.
- Mutation command (`add`) can create noisy config drift if not idempotent; mitigate with stable upsert keys and formatting rules.
- `PROJECT_PATH` misconfiguration can point at wrong repository; mitigate with `--explain` style diagnostics or verbose path reporting.
- Teams may expect TOML to remove built-ins; contract must clearly state built-ins are mandatory defaults.

## Rollback plan
- Keep implementation split into small commits: scaffold, resolver, TOML parser, mutation command, integrations.
- If TOML extension causes regressions, disable `add` command and TOML merge path first while keeping built-in resolver functional.
- If completion/wrapper updates regress shell tests, revert integration files (`completions/*`, `wrappers/agent-docs`) independently.
- If resolver semantics regress, revert to Sprint 2 built-in-only behavior and re-run required checks before reintroducing extensions.
