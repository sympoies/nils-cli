# Plan: agent-docs resolve checklist output format

## Overview
This plan adds a first-class `checklist` output mode to `agent-docs resolve` so preflight
required-doc verification can use a stable, machine-parseable contract without ad-hoc text
parsing. The change preserves existing `text` and `json` behavior and keeps strict/non-strict exit
code semantics unchanged. The implementation focuses on a resolve-scoped format contract, coverage
for parity-sensitive edge cases, and completion/docs updates so operators can adopt it safely.

## Scope
- In scope:
  - Add `--format checklist` support to `agent-docs resolve`.
  - Define deterministic checklist output lines (`filename`, `status`, `path`) plus summary footer.
  - Add/update tests for output contract stability, strict behavior, and extension-doc merge paths.
  - Update shell completions and `agent-docs` README to reflect new format.
  - Update project docs that show `agent-docs resolve` verification examples where appropriate.
- Out of scope:
  - Adding checklist format to `contexts`, `baseline`, or `scaffold-baseline`.
  - Changing built-in context semantics or strict fallback policy.
  - Editing external home-level runbooks outside this repository.

## Assumptions (if any)
1. Existing consumers that parse `text` output can remain unchanged because default format remains
   `text`.
2. Checklist mode should include required documents only, preserving original resolve ordering.
3. The output contract must remain deterministic across repeated runs with identical filesystem
   state.

## Sprint 1: Resolve format contract and renderer foundation
**Goal**: Add a resolve-specific checklist format path with deterministic rendering while preserving
existing command behavior.
**Demo/Validation**:
- Command(s):
  - `cargo run -q -p agent-docs -- resolve --context startup --format checklist`
  - `cargo run -q -p agent-docs -- resolve --context project-dev --format checklist --strict`
- Verify:
  - Output includes `REQUIRED_DOCS_BEGIN ...` and `REQUIRED_DOCS_END ...` markers.
  - Marker fields include `context`, `mode`, and summary counts.

**Parallelization notes**:
- `Task 1.1` should land first because it defines the CLI/model contract.
- `Task 1.2` depends on `Task 1.1`.
- `Task 1.3` depends on `Task 1.2`.

### Task 1.1: Introduce resolve-scoped output format enum and CLI wiring
- **Location**:
  - `crates/agent-docs/src/model.rs`
  - `crates/agent-docs/src/cli.rs`
  - `crates/agent-docs/src/lib.rs`
- **Description**: Add a resolve-specific format enum that supports `text|json|checklist`, wire it
  through `ResolveArgs`, and keep other subcommands on their existing `text|json` format contract.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - `resolve --help` shows `checklist` as an allowed format value.
  - Non-resolve commands keep their existing format surface and behavior.
  - No regression to strict exit-code behavior wiring.
- **Validation**:
  - `cargo run -q -p agent-docs -- resolve --help | rg -n "checklist"`
  - `if cargo run -q -p agent-docs -- baseline --help | rg -q "checklist"; then exit 1; fi`

### Task 1.2: Implement deterministic checklist renderer for resolve reports
- **Location**:
  - `crates/agent-docs/src/output.rs`
  - `crates/agent-docs/src/model.rs`
- **Description**: Implement checklist rendering with this shape: begin marker, one required-doc
  line per document (`<filename> status=<...> path=<...>`), and end marker summary counts. Ensure
  filename uses basename extraction, ordering matches resolver output, and mode maps to
  `strict|non-strict`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Only required docs are emitted in checklist lines.
  - Checklist lines preserve resolver order and status fidelity.
  - End summary counts match emitted required-doc counts and strict mode.
- **Validation**:
  - `cargo run -q -p agent-docs -- resolve --context startup --format checklist | rg -n "REQUIRED_DOCS_BEGIN|REQUIRED_DOCS_END|AGENTS"`
  - `cargo run -q -p agent-docs -- resolve --context project-dev --format checklist --strict | rg -n "mode=strict|REQUIRED_DOCS_END"`
  - `out="$(cargo run -q -p agent-docs -- resolve --context startup --format checklist)"; req_lines="$(printf '%s\n' "$out" | rg -c '^.+ status=(present|missing) path=/')"; required="$(printf '%s\n' "$out" | sed -n 's/^REQUIRED_DOCS_END required=\\([0-9]\\+\\) present=.*/\\1/p')"; test "$req_lines" -eq "$required"`

### Task 1.3: Add rendering-focused tests for checklist contract and stability
- **Location**:
  - `crates/agent-docs/tests/resolve_builtin.rs`
  - `crates/agent-docs/tests/common.rs`
- **Description**: Extend builtin resolve coverage to assert checklist marker fields, required-line
  ordering, filename/status/path contract, and deterministic output across repeated renders.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Checklist output tests cover all built-in contexts.
  - Determinism checks validate repeated render equality for checklist mode.
  - Missing-required paths still preserve strict/non-strict exit semantics.
- **Validation**:
  - `cargo test -p agent-docs resolve_builtin -- --nocapture`

## Sprint 2: Integration parity, completions, and docs
**Goal**: Ensure checklist mode is fully integrated (TOML extensions + shell UX + docs) without
regressing existing flows.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agent-docs resolve_toml -- --nocapture`
  - `zsh -f tests/zsh/completion.test.zsh`
- Verify:
  - Extension docs appear correctly in checklist mode.
  - Completion suggestions include the new resolve format.
  - README examples match real command/output behavior.

**Parallelization notes**:
- `Task 2.1` depends on Sprint 1 output tests.
- `Task 2.2` can run in parallel with `Task 2.1` once `Task 1.1` lands.
- `Task 2.3` depends on `Task 1.2` but is otherwise independent.

### Task 2.1: Add CLI integration tests for checklist with extension config and strict paths
- **Location**:
  - `crates/agent-docs/tests/resolve_toml.rs`
  - `crates/agent-docs/tests/common.rs`
- **Description**: Add integration tests that execute the binary with `--format checklist` and
  assert merged built-in + TOML required docs, malformed-config behavior, and strict failure path
  parity.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Checklist integration tests assert extension doc inclusion and stable status fields.
  - Strict mode still exits non-zero when required docs are missing.
  - Config parse errors still return config exit code and stderr diagnostics.
- **Validation**:
  - `cargo test -p agent-docs resolve_toml -- --nocapture`
  - `cargo test -p agent-docs resolve_builtin_strict_and_non_strict_have_different_exit_codes_for_missing_required_docs -- --nocapture`

### Task 2.2: Update zsh/bash completions and completion regression coverage
- **Location**:
  - `completions/zsh/_agent-docs`
  - `completions/bash/agent-docs`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add `checklist` to resolve format completion values while preserving existing
  format values for other subcommands. Extend tests so completion drift is caught in CI.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Zsh completion exposes `checklist` for resolve format.
  - Bash completion exposes `checklist` in resolve format suggestions.
  - Completion regression test fails if `checklist` is removed from resolve suggestions.
- **Validation**:
  - `rg -n -- "format:\\(text json checklist\\)|formats=\\(text json checklist\\)" completions/zsh/_agent-docs completions/bash/agent-docs`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 2.3: Update `agent-docs` README for checklist usage and output contract
- **Location**:
  - `crates/agent-docs/README.md`
- **Description**: Document `resolve --format checklist` flags, include a concrete checklist output
  example, and clarify when operators should prefer checklist vs text/json modes.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - README command flags include checklist format for resolve.
  - Output contract section includes begin/end marker examples and field descriptions.
  - At least one copy-paste verification command uses checklist mode.
- **Validation**:
  - `rg -n "resolve --context .* --format checklist|REQUIRED_DOCS_BEGIN|REQUIRED_DOCS_END" crates/agent-docs/README.md`

## Sprint 3: Hardening, downstream adoption, and release gate
**Goal**: Add dedicated parseability hardening tests, align downstream examples, and clear required
delivery checks.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agent-docs resolve_checklist -- --nocapture`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - Checklist parsing contract is stable for preflight automation use.
  - Project-level guidance references checklist mode where verification speed matters.
  - Workspace-required checks pass before merge.

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can run in parallel.
- `Task 3.3` depends on both to finalize release readiness.

### Task 3.1: Add dedicated checklist contract tests for machine parsing
- **Location**:
  - `crates/agent-docs/tests/resolve_checklist.rs`
  - `crates/agent-docs/tests/common.rs`
- **Description**: Add integration tests that parse checklist output into structured rows and
  verify marker metadata, required-doc rows, ordering, and summary consistency across contexts.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests validate checklist parsing for `startup` and `project-dev`.
  - Summary counts exactly match parsed required-doc rows.
  - Repeated command runs produce identical checklist output under stable fixtures.
- **Validation**:
  - `cargo test -p agent-docs resolve_checklist -- --nocapture`
  - `diff <(cargo run -q -p agent-docs -- resolve --context startup --format checklist) <(cargo run -q -p agent-docs -- resolve --context startup --format checklist)`

### Task 3.2: Align repository verification examples with checklist mode
- **Location**:
  - `BINARY_DEPENDENCIES.md`
  - `README.md`
- **Description**: Update repo-level examples to show checklist mode for fast required-doc
  verification while keeping existing text/json examples where full diagnostics are needed.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 2
- **Acceptance criteria**:
  - At least one project-level verification snippet uses checklist mode.
  - Existing documentation remains internally consistent with command behavior.
  - No conflicting examples between root docs and crate README.
- **Validation**:
  - `rg -n "agent-docs resolve --context .* --format checklist" BINARY_DEPENDENCIES.md README.md crates/agent-docs/README.md`

### Task 3.3: Run full required checks and finalize delivery readiness
- **Location**:
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `crates/agent-docs/Cargo.toml`
- **Description**: Run required repository checks and ensure all checklist-related tests, docs, and
  completions pass together before merge.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Required repo checks pass without suppressing warnings.
  - Agent-docs targeted tests pass with checklist mode included.
  - No completion regressions in zsh test suite.
- **Validation**:
  - `cargo test -p agent-docs --tests`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - Extend rendering-level assertions in `resolve_builtin` for checklist markers and ordering.
  - Validate enum parsing/format routing behavior in command-dispatch unit coverage.
- Integration:
  - Add binary-level checklist tests using fixture workspaces with/without extension TOML.
  - Add strict/non-strict parity checks to confirm unchanged exit-code semantics.
- E2E/manual:
  - Manually run `agent-docs resolve --format checklist` for startup and project-dev contexts.
  - Run zsh completion test to verify command-line ergonomics.

## Risks & gotchas
- If checklist is added to a shared format enum without command scoping, other subcommands may
  accidentally accept unsupported format values.
- Completion scripts can drift from clap argument behavior unless tests assert format values.
- Operators may over-rely on checklist output and miss optional-doc diagnostics available in text
  mode.

## Rollback plan
- Revert resolve checklist enum/CLI wiring and renderer changes while retaining text/json paths.
- Remove checklist-specific tests and completion entries if contract proves unstable.
- Restore README and repo docs to pre-checklist examples and re-run required checks to confirm
  baseline parity.
