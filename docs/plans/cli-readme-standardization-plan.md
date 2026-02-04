# Plan: Standardize CLI README format

## Overview
Standardize all CLI crate READMEs to match the `git-cli` style (sections and tone), while allowing
per-CLI optional sections as needed. Remove legacy port/upstream references and focus on current
features only. Changes are documentation-only and scoped to CLI crates that have `src/main.rs`.

## Scope
- In scope:
  - CLI crate READMEs for crates with `src/main.rs`.
  - Consistent section structure: Overview, Usage, Commands, and optional sections (Aliases,
    Exit codes, Dependencies, Environment, Examples).
- Out of scope:
  - Non-CLI crates (`nils-common`, `nils-term`, `nils-test-support`, `api-testing-core`).
  - Root `README.md`.
  - Code changes or CLI behavior changes.

## Assumptions (if any)
1. Scope = all CLI crates with `crates/*/src/main.rs`.
2. Format consistency: match `git-cli` style but allow per-CLI sections to be added/removed.
3. README language stays English.

## Sprint 1: Inventory + Template
**Goal**: Identify all CLI READMEs and define the shared README skeleton and per-CLI checklist.
**Demo/Validation**:
- Command(s): `rg --files -g 'main.rs' crates`
- Verify: CLI inventory list matches README list; draft skeleton is ready to apply.

### Task 1.1: Build CLI inventory
- **Location**:
  - `Cargo.toml`
  - `crates/git-cli/src/main.rs`
  - `crates/git-scope/src/main.rs`
  - `crates/git-summary/src/main.rs`
  - `crates/git-lock/src/main.rs`
  - `crates/codex-cli/src/main.rs`
  - `crates/fzf-cli/src/main.rs`
  - `crates/semantic-commit/src/main.rs`
  - `crates/plan-tooling/src/main.rs`
  - `crates/api-test/src/main.rs`
  - `crates/api-rest/src/main.rs`
  - `crates/api-gql/src/main.rs`
  - `crates/image-processing/src/main.rs`
  - `crates/cli-template/src/main.rs`
- **Description**: Enumerate CLI crates by `src/main.rs` and map each to its README.
- **Dependencies**:
  - none
- **Complexity**: 1
- **Acceptance criteria**:
  - A definitive list of CLI READMEs is produced and used for updates.
- **Validation**:
  - `rg --files -g 'main.rs' crates` matches README targets.

### Task 1.2: Define README skeleton + optional sections
- **Location**:
  - `crates/git-cli/README.md`
  - `docs/plans/cli-readme-standardization-plan.md`
- **Description**: Derive a canonical README skeleton (Overview, Usage, Commands) and
  permissible optional sections (Aliases, Exit codes, Dependencies, Environment, Examples).
Skeleton (canonical):
- `## Overview`
- `## Usage`
- `## Commands`

Optional sections (use as needed):
- `## Aliases`
- `## Exit codes`
- `## Dependencies`
- `## Environment`
- `## Examples`
- **Dependencies**:
  - Task 1.1
- **Complexity**: 1
- **Acceptance criteria**:
  - Skeleton is documented and ready to apply without upstream/port references.
- **Validation**:
  - Skeleton includes the three core sections and optional section list.

## Sprint 2: Git + Core Dev CLIs
**Goal**: Update git-related and core dev-tool CLIs to the new README style.
**Demo/Validation**:
- Command(s): `rg -n "upstream|port|parity|fixtures" crates/*/README.md`
- Verify: No legacy port/upstream references remain in targeted READMEs.

### Task 2.1: Update git-related CLIs
- **Location**:
  - `crates/git-cli/README.md`
  - `crates/git-scope/README.md`
  - `crates/git-summary/README.md`
  - `crates/git-lock/README.md`
- **Description**: Rewrite each README to the new format; document current commands, flags, and
  dependencies only.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - Each README includes Overview/Usage/Commands sections.
  - No port/upstream language remains.
  - Command list matches current CLI behavior.
- **Validation**:
  - Manual review of sections and `rg` check for legacy terms.

### Task 2.2: Update core dev CLIs
- **Location**:
  - `crates/codex-cli/README.md`
  - `crates/fzf-cli/README.md`
  - `crates/semantic-commit/README.md`
  - `crates/plan-tooling/README.md`
- **Description**: Rewrite each README using the skeleton; capture usage, commands, options,
  exit codes, and dependencies as relevant.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - Same as Task 2.1.
- **Validation**:
  - Manual review of sections and `rg` check for legacy terms.

## Sprint 3: API + Utility CLIs
**Goal**: Update remaining CLI READMEs to the standardized style.
**Demo/Validation**:
- Command(s): `rg -n "^## (Overview|Usage|Commands)" crates/*/README.md`
- Verify: Each targeted README contains core sections.

### Task 3.1: Update API testing CLIs
- **Location**:
  - `crates/api-test/README.md`
  - `crates/api-rest/README.md`
  - `crates/api-gql/README.md`
- **Description**: Rewrite each README to the new format, documenting commands, flags, inputs,
  and outputs.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - Same as Task 2.1.
- **Validation**:
  - Manual review + `rg` check for legacy terms.

### Task 3.2: Update utility/template CLIs
- **Location**:
  - `crates/image-processing/README.md`
  - `crates/cli-template/README.md`
- **Description**: Rewrite each README with the skeleton and per-CLI optional sections.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 1
- **Acceptance criteria**:
  - Same as Task 2.1.
- **Validation**:
  - Manual review + `rg` check for legacy terms.

## Testing Strategy
- Unit: N/A (docs-only).
- Integration: N/A.
- E2E/manual:
  - `rg -n "upstream|port|parity|fixtures" crates/*/README.md` returns no matches.
  - Spot-check each README for Overview/Usage/Commands and correct command lists.

## Risks & gotchas
- README command lists may drift from implementation if not cross-checked with CLI help output.
- Some CLIs may need additional sections (environment variables, config files) to avoid losing
  important usage details.

## Rollback plan
- Revert the updated README files via git if content is inaccurate or incomplete.
