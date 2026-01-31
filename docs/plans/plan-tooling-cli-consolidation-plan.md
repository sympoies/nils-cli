# Plan: Plan tooling CLI consolidation

## Overview

Consolidate the current Plan Format v1 tooling scripts (`plan_to_json.sh`, `validate_plans.sh`, `plan_batches.sh`, `scaffold_plan.sh`) into a single CLI entrypoint with subcommands. Keep the plan markdown format and JSON schema stable, and preserve exit codes plus `error:`-style diagnostics so existing workflows keep working. Maintain the existing `.sh` entrypoints as thin wrappers for backwards compatibility while updating docs and skills to prefer the new CLI.

Source scripts (current): `/Users/terry/.config/codex-kit/skills/workflows/plan/plan-tooling/scripts`.

## Scope
- In scope: A single CLI with subcommands for JSON export, linting, batch computation, and scaffolding; wrappers for existing entrypoints; docs + skill updates; tests that cover both the new CLI and wrapper parity.
- Out of scope: Changing Plan Format v1, changing `/execute-plan-parallel` behavior, adding non-trivial new plan semantics (new fields, new dependency rules), or introducing non-stdlib runtime dependencies.

## Assumptions (if any)
1. The unified CLI is implemented in Python 3 (stdlib only) to match the repo’s existing bash + embedded-python style and avoid new toolchain dependencies.
2. Existing entrypoints remain available and behave the same (exit codes, stdout/stderr structure) by delegating to the new CLI; no deprecation warnings are emitted by default to avoid breaking current consumers and tests.
3. `CODEX_HOME` may be unset; the CLI derives repo root from its own location when running from a checked-out repo, matching the current scripts’ behavior.

## Compatibility contract (parity targets)

The unified CLI and wrapper scripts must preserve the current contracts for these entrypoints:

- `skills/workflows/plan/plan-tooling/scripts/plan_to_json.sh`
  - Flags: `--file <path>` (required), `--sprint <n>`, `--pretty`, `-h/--help`
  - Success: JSON to stdout, exit 0
  - Parse errors: `error:` lines to stderr, exit 1
  - Usage errors: exit 2; stderr behavior must match the existing script per-case (some cases print usage, others print a single `plan_to_json:` line)
- `skills/workflows/plan/plan-tooling/scripts/validate_plans.sh`
  - Flags: `--file <path>` (repeatable), `-h/--help`
  - Success: no stdout/stderr output, exit 0
  - Validation errors: `error:` lines to stderr, exit 1
  - Usage errors: exit 2; stderr behavior must match the existing script per-case (unknown args currently print a single `validate_plans:` line)
- `skills/workflows/plan/plan-tooling/scripts/plan_batches.sh`
  - Flags: `--file <path>` (required), `--sprint <n>` (required), `--format json|text`, `-h/--help`
  - Success: JSON or text to stdout, exit 0
  - Runtime errors (including cycles): `error:` lines to stderr, exit 1
  - Usage errors: exit 2; stderr behavior must match the existing script per-case (for example, an invalid `--sprint` value prints an `error:` line and exits 2)
- `skills/workflows/plan/plan-tooling/scripts/scaffold_plan.sh`
  - Flags: `--slug <kebab-case>` or `--file <path>`, optional `--title <text>`, `--force`, `-h/--help`
  - Success: prints `created: <path>` to stdout, exit 0
  - Runtime errors: prints `scaffold_plan: error: ...` to stderr, exit 1
  - Usage errors: prints a `scaffold_plan:` line plus usage, exit 2

## Sprint 1: CLI parity baseline (parser + to-json)
**Goal**: Introduce a single CLI entrypoint and shared Plan Format v1 parser, and make `to-json` behavior match `plan_to_json.sh`.
**Demo/Validation**:
- Command(s): `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py to-json --file tests/fixtures/plan/valid-plan.md --pretty | python3 -m json.tool`
- Verify: JSON contains `title`, `file`, `sprints[0].tasks[*].id`, and correct `start_line` values.

### Task 1.1: Add unified plan-tooling CLI with shared parser and `to-json`
- **Location**:
  - `skills/workflows/plan/plan-tooling/scripts/plan_tooling.py`
- **Description**: Create a single executable Python CLI with subcommands and shared parsing code for Plan Format v1; implement `to-json` as the canonical replacement for `plan_to_json.sh` (same schema, same `start_line` semantics, same `--sprint` filtering behavior, and optional `--pretty` output).
- **Dependencies**: none
- **Complexity**: 6
- **Acceptance criteria**:
  - `plan_tooling.py to-json --file tests/fixtures/plan/valid-plan.md` emits valid JSON with the expected top-level keys and task IDs.
  - `plan_tooling.py to-json --file tests/fixtures/plan/valid-plan.md --sprint 1` returns exactly one sprint.
  - `plan_tooling.py to-json --file tests/fixtures/plan/valid-plan.md --pretty` emits JSON with indent=2.
  - Parse errors exit 1 and print `error:` lines to stderr.
  - Usage errors exit 2 and match the compatibility contract for `plan_to_json.sh` stderr behavior.
- **Validation**:
  - `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py to-json --file tests/fixtures/plan/valid-plan.md --pretty | python3 -m json.tool`
  - `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py to-json --file tests/fixtures/plan/valid-plan.md --sprint 1 >/dev/null`

### Task 1.2: Add tests for the new CLI `to-json` subcommand
- **Location**:
  - `tests/test_plan_tooling_cli.py`
- **Description**: Add pytest coverage that runs the new `plan_tooling.py to-json` command against the existing plan fixtures and asserts schema basics, task IDs, and `--sprint` filtering.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `tests/test_plan_tooling_cli.py` asserts `to-json` succeeds on `tests/fixtures/plan/valid-plan.md` and emits parseable JSON.
  - `tests/test_plan_tooling_cli.py` asserts `to-json` fails with exit 1 on a missing plan file and prints an `error:` line.
- **Validation**:
  - `scripts/test.sh -q tests/test_plan_tooling_cli.py`

## Sprint 2: Implement remaining subcommands (validate, batches, scaffold)
**Goal**: Implement `validate`, `batches`, and `scaffold` subcommands with behavior equivalent to the existing scripts.
**Demo/Validation**:
- Command(s): `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py validate --file tests/fixtures/plan/valid-plan.md`
- Verify: Exit 0 and no output on success; invalid fixtures produce exit 1 with `error:` lines.

### Task 2.1: Implement `validate` subcommand (Plan Format v1 lint)
- **Location**:
  - `skills/workflows/plan/plan-tooling/scripts/plan_tooling.py`
  - `tests/test_plan_tooling_cli.py`
- **Description**: Implement `validate` to lint one or more plan files using the same rules as `validate_plans.sh`, including placeholder detection, required task fields, dependency ID validation, and default discovery of `docs/plans/*-plan.md` via git (with a filesystem fallback).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 7
- **Acceptance criteria**:
  - `plan_tooling.py validate` exits 0 when all tracked `docs/plans/*-plan.md` files are valid.
  - `plan_tooling.py validate --file tests/fixtures/plan/invalid-plan.md` exits 1 and reports a missing Validation field.
  - Usage errors exit 2 and match the compatibility contract for `validate_plans.sh` stderr behavior.
- **Validation**:
  - `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py validate --file tests/fixtures/plan/valid-plan.md`
  - `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py validate --file tests/fixtures/plan/invalid-plan.md; test $? -eq 1`
  - `scripts/test.sh -q tests/test_plan_tooling_cli.py`

### Task 2.2: Implement `batches` subcommand (parallel dependency layers)
- **Location**:
  - `skills/workflows/plan/plan-tooling/scripts/plan_tooling.py`
  - `tests/test_plan_tooling_cli.py`
- **Description**: Implement `batches` as a replacement for `plan_batches.sh`, including topo-layer output for in-sprint dependencies, cycle detection, reporting external blockers, and emitting both JSON and text formats with stable ordering.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `plan_tooling.py batches --file tests/fixtures/plan/valid-plan.md --sprint 1` emits JSON where `batches` equals `[["Task 1.1"], ["Task 1.2", "Task 1.3"]]`.
  - The JSON output includes `blocked_by_external` and `conflict_risk` keys (even when empty).
  - `plan_tooling.py batches --format text ...` prints Batch sections and lists task IDs.
  - Dependency cycles result in exit 1 with a clear `error:` message.
- **Validation**:
  - `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py batches --file tests/fixtures/plan/valid-plan.md --sprint 1 | python3 -m json.tool`
  - `scripts/test.sh -q tests/test_plan_tooling_cli.py`

### Task 2.3: Implement `scaffold` subcommand (create plan from template)
- **Location**:
  - `skills/workflows/plan/plan-tooling/scripts/plan_tooling.py`
  - `tests/test_plan_tooling_cli.py`
- **Description**: Implement `scaffold` as a replacement for `scaffold_plan.sh`, including slug validation, `--file` output, `--title` replacement for the first heading, and `--force` overwrite behavior.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `plan_tooling.py scaffold --slug plan-tooling-cli-consolidation-test --title "Test plan"` creates `docs/plans/plan-tooling-cli-consolidation-test-plan.md` when it does not already exist.
  - Re-running the command without `--force` fails with exit 1 and a clear error.
  - Re-running the command with `--force` overwrites the file and exits 0.
  - On success, the command prints `created: ...` to stdout.
- **Validation**:
  - `python3 skills/workflows/plan/plan-tooling/scripts/plan_tooling.py scaffold --file out/plan-tooling-cli-consolidation-test-plan.md --title \"Test plan\" --force`

### Task 2.4: Add parity fixtures and tests for edge cases
- **Location**:
  - `tests/fixtures/plan/edge-placeholder.md`
  - `tests/fixtures/plan/edge-location-invalid.md`
  - `tests/fixtures/plan/edge-cycle.md`
  - `tests/fixtures/plan/edge-external-blockers.md`
  - `tests/test_plan_tooling_cli.py`
- **Description**: Add plan fixtures and direct CLI tests that lock in edge-case behavior across all subcommands: dependency normalization (including comma-separated lists), placeholder detection, Location path rules, Complexity type/range validation, external blockers, `conflict_risk`, and dependency cycles.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - New fixtures cover at least: placeholder token errors, Location path rule errors, a cycle in dependencies, and an external dependency reference.
  - `tests/test_plan_tooling_cli.py` asserts exit codes and `error:` stderr lines for each added failure fixture.
- **Validation**:
  - `scripts/test.sh -q tests/test_plan_tooling_cli.py`

## Sprint 3: Wire wrappers, docs, and migration
**Goal**: Keep existing entrypoints stable by delegating to the unified CLI, and update docs/skills/tests to prefer the new CLI.
**Demo/Validation**:
- Command(s): `scripts/check.sh --plans` and `scripts/test.sh -q tests/test_plan_scripts.py`
- Verify: All existing plan script smoke tests continue to pass, and the unified CLI is documented as the preferred interface.

### Task 3.1: Convert existing `.sh` entrypoints into wrappers over the unified CLI
- **Location**:
  - `skills/workflows/plan/plan-tooling/scripts/plan_to_json.sh`
  - `skills/workflows/plan/plan-tooling/scripts/validate_plans.sh`
  - `skills/workflows/plan/plan-tooling/scripts/plan_batches.sh`
  - `skills/workflows/plan/plan-tooling/scripts/scaffold_plan.sh`
- **Description**: Replace the current bash + embedded-python implementations with thin wrapper scripts that invoke `plan_tooling.py` subcommands, preserving CLI flags, exit codes, and stdout/stderr behavior.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
  - Task 2.4
- **Complexity**: 4
- **Acceptance criteria**:
  - `tests/test_plan_scripts.py` continues to pass without modifications to expected stdout/stderr patterns.
  - `scripts/check.sh --plans` continues to run plan validation successfully.
- **Validation**:
  - `scripts/test.sh -q tests/test_plan_scripts.py`
  - `scripts/check.sh --plans`

### Task 3.2: Update docs and skills to prefer the unified CLI
- **Location**:
  - `skills/workflows/plan/plan-tooling/SKILL.md`
  - `skills/workflows/plan/create-plan/SKILL.md`
  - `skills/workflows/plan/create-plan-rigorous/SKILL.md`
  - `skills/workflows/plan/execute-plan-parallel/SKILL.md`
  - `docs/runbooks/plan-workflow.md`
  - `docs/runbooks/skills/TOOLING_INDEX_V2.md`
  - `docs/plans/FORMAT.md`
  - `docs/plans/TOOLCHAIN.md`
  - `scripts/README.md`
- **Description**: Update references to the plan tooling entrypoints so the primary examples use the unified CLI, while still mentioning the legacy wrapper scripts as supported entrypoints.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Runbooks and docs show the unified CLI usage for linting, JSON export, batching, and scaffolding.
  - Skill docs still list supported entrypoints and remain consistent with the actual filesystem layout.
- **Validation**:
  - `scripts/check.sh --lint --contracts --plans`

### Task 3.3: Add coverage for unified CLI end-to-end and run full checks
- **Location**:
  - `tests/test_plan_tooling_cli.py`
  - `scripts/check.sh`
- **Description**: Extend tests so the unified CLI has direct coverage for `validate`, `batches`, and `scaffold` behavior (in addition to `to-json`), and run the repo’s full check suite to ensure wrappers and docs changes do not break other workflows.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - `tests/test_plan_tooling_cli.py` covers all four subcommands and passes consistently.
  - `scripts/check.sh --all` exits 0.
- **Validation**:
  - `scripts/check.sh --all`

## Testing Strategy
- Unit: Add focused parser/validator tests via fixtures under `tests/fixtures/plan/` and assert exact task IDs, dependency normalization, and placeholder detection.
- Integration: Run both wrapper scripts and the unified CLI in pytest to ensure parity for exit codes and output.
- E2E/manual: Use `docs/runbooks/plan-workflow.md` steps with the unified CLI and confirm `/execute-plan-parallel` can still consume the same plan files.

## Risks & gotchas
- Output drift: wrapper scripts must remain silent on success (stdout/stderr) to avoid breaking existing smoke tests and higher-level workflows.
- Repo root discovery: `CODEX_HOME` auto-detection must match current behavior to keep repo-relative paths stable in JSON output.
- Git dependency: `validate` relies on git for tracked-file discovery; behavior should remain stable when running with explicit `--file` values.

## Rollback plan
- Keep changes in small commits: first add the new CLI, then switch wrappers, then update docs.
- If wrappers regress: revert wrapper changes first, then rerun `scripts/check.sh --plans` and `scripts/test.sh -q tests/test_plan_scripts.py` to confirm the old entrypoints are restored.
- If docs/skills updates cause confusion or drift: revert the doc/skill commits next while keeping the unified CLI available for iterative fixes.
- If the unified CLI regresses parsing semantics: keep wrappers on the old implementations temporarily, add/extend fixtures under `tests/fixtures/plan/`, and only re-enable delegation after `scripts/check.sh --all` is green.
